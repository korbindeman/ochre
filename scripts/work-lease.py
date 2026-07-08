#!/usr/bin/env python3
"""Work leases — defer compile/test until no agent is actively editing.

Each session claims a lease at slice start and releases before verify. Build and
test recipes wait until every lease is gone (or stale). Cargo's own file lock
handles simultaneous compiles; this gate avoids verifying a moving tree.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path


def repo_root() -> Path:
    env = os.environ.get("BROWSER_ROOT")
    if env:
        return Path(env).resolve()
    return Path(__file__).resolve().parent.parent


def lease_dir(root: Path) -> Path:
    path = root / "target" / ".work-leases"
    path.mkdir(parents=True, exist_ok=True)
    return path


def session_cookie_path(root: Path) -> Path:
    return lease_dir(root) / ".session-cookie"


def resolve_session_id(root: Path, *, create: bool) -> str | None:
    if env := os.environ.get("BROWSER_WORK_SESSION"):
        return env

    cookie = session_cookie_path(root)
    if cookie.exists():
        sid = cookie.read_text().strip()
        if sid:
            return sid

    if not create:
        return None

    sid = f"{socket.gethostname()}-{uuid.uuid4().hex[:8]}"
    cookie.write_text(f"{sid}\n")
    return sid


def heartbeat_ttl() -> float:
    return float(os.environ.get("BROWSER_WORK_LEASE_TTL", "900"))


def poll_interval() -> float:
    return float(os.environ.get("BROWSER_WORK_LEASE_POLL", "5"))


@dataclass
class Lease:
    session_id: str
    pid: int
    task: str
    hostname: str
    claimed_at: float
    heartbeat_at: float
    path: Path

    def age_s(self) -> float:
        return time.time() - self.heartbeat_at

    def summary(self) -> str:
        task = f' "{self.task}"' if self.task else ""
        return f"{self.session_id}{task} (idle {int(self.age_s())}s)"


def read_lease(path: Path) -> Lease | None:
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    try:
        return Lease(
            session_id=data["session_id"],
            pid=int(data["pid"]),
            task=str(data.get("task", "")),
            hostname=str(data.get("hostname", "")),
            claimed_at=float(data.get("claimed_at", 0)),
            heartbeat_at=float(data["heartbeat_at"]),
            path=path,
        )
    except (KeyError, TypeError, ValueError):
        return None


def is_stale(lease: Lease) -> bool:
    return lease.age_s() > heartbeat_ttl()


def prune_stale(root: Path) -> None:
    for path in lease_dir(root).glob("*.json"):
        lease = read_lease(path)
        if lease is None or is_stale(lease):
            path.unlink(missing_ok=True)


def active_leases(root: Path) -> list[Lease]:
    prune_stale(root)
    leases: list[Lease] = []
    for path in sorted(lease_dir(root).glob("*.json")):
        lease = read_lease(path)
        if lease is not None:
            leases.append(lease)
    return leases


def own_lease(root: Path) -> Lease | None:
    sid = resolve_session_id(root, create=False)
    if sid is None:
        return None
    for lease in active_leases(root):
        if lease.session_id == sid:
            return lease
    return None


def write_lease(root: Path, sid: str, task: str) -> Lease:
    now = time.time()
    lease = Lease(
        session_id=sid,
        pid=os.getppid(),
        task=task,
        hostname=socket.gethostname(),
        claimed_at=now,
        heartbeat_at=now,
        path=lease_dir(root) / f"{sid}.json",
    )
    lease.path.write_text(
        json.dumps(
            {
                "session_id": lease.session_id,
                "pid": lease.pid,
                "task": lease.task,
                "hostname": lease.hostname,
                "claimed_at": lease.claimed_at,
                "heartbeat_at": lease.heartbeat_at,
            }
        )
    )
    return lease


def cmd_begin(task: str) -> int:
    root = repo_root()
    sid = resolve_session_id(root, create=True)
    assert sid is not None

    existing = own_lease(root)
    if existing is not None:
        existing.path.write_text(
            json.dumps(
                {
                    "session_id": existing.session_id,
                    "pid": existing.pid,
                    "task": task or existing.task,
                    "hostname": existing.hostname,
                    "claimed_at": existing.claimed_at,
                    "heartbeat_at": time.time(),
                }
            )
        )
        print(f"[work-lease] refreshed ({sid})")
        return 0

    lease = write_lease(root, sid, task)
    label = f" — {task}" if task else ""
    print(f"[work-lease] claimed {lease.session_id}{label}")
    return 0


def cmd_touch() -> int:
    root = repo_root()
    lease = own_lease(root)
    if lease is None:
        print(
            "[work-lease] no lease for this session; run `just work-begin`",
            file=sys.stderr,
        )
        return 1

    lease.path.write_text(
        json.dumps(
            {
                "session_id": lease.session_id,
                "pid": lease.pid,
                "task": lease.task,
                "hostname": lease.hostname,
                "claimed_at": lease.claimed_at,
                "heartbeat_at": time.time(),
            }
        )
    )
    return 0


def cmd_end() -> int:
    root = repo_root()
    lease = own_lease(root)
    if lease is None:
        print("[work-lease] no lease for this session")
        return 0
    lease.path.unlink(missing_ok=True)
    cookie = session_cookie_path(root)
    if (
        not os.environ.get("BROWSER_WORK_SESSION")
        and cookie.exists()
        and cookie.read_text().strip() == lease.session_id
    ):
        cookie.unlink(missing_ok=True)
    print(f"[work-lease] released {lease.session_id}")
    return 0


def cmd_status() -> int:
    root = repo_root()
    leases = active_leases(root)
    if not leases:
        print("[work-lease] idle — no active sessions")
        return 0
    print(f"[work-lease] {len(leases)} active session(s):")
    for lease in leases:
        print(f"  • {lease.summary()}")
    return 0


def wait_for_idle(root: Path) -> int:
    if os.environ.get("BROWSER_WORK_LEASE", "1").lower() in ("0", "false", "no"):
        return 0

    own = own_lease(root)
    if own is not None:
        print(
            "[work-lease] this session still holds a lease — run `just work-end` before building",
            file=sys.stderr,
        )
        return 1

    timeout = os.environ.get("BROWSER_WORK_LEASE_TIMEOUT")
    deadline = time.monotonic() + float(timeout) if timeout else None
    poll_s = poll_interval()

    while True:
        leases = active_leases(root)
        if not leases:
            return 0

        if deadline is not None and time.monotonic() >= deadline:
            print(
                "[work-lease] timed out waiting for active sessions to finish",
                file=sys.stderr,
            )
            return 75

        lines = ", ".join(lease.summary() for lease in leases)
        print(
            f"[work-lease] waiting — {len(leases)} session(s) still editing: {lines}",
            file=sys.stderr,
        )
        time.sleep(poll_s)


def cmd_run(argv: list[str]) -> int:
    if not argv:
        print("usage: work-lease.py run <command> [args...]", file=sys.stderr)
        return 2

    root = repo_root()
    rc = wait_for_idle(root)
    if rc != 0:
        return rc
    return subprocess.call(argv)


def usage() -> None:
    print(
        "usage: work-lease.py {begin,touch,end,status,wait,run} [args...]",
        file=sys.stderr,
    )


def main() -> int:
    if len(sys.argv) < 2:
        usage()
        return 2

    cmd = sys.argv[1]
    if cmd == "begin":
        task = " ".join(sys.argv[2:]) if len(sys.argv) > 2 else ""
        return cmd_begin(task)
    if cmd == "touch":
        return cmd_touch()
    if cmd == "end":
        return cmd_end()
    if cmd == "status":
        return cmd_status()
    if cmd == "wait":
        return wait_for_idle(repo_root())
    if cmd == "run":
        return cmd_run(sys.argv[2:])

    usage()
    return 2


if __name__ == "__main__":
    sys.exit(main())
