# Releasing a friends build

A release is a GitHub pre-release built by `.github/workflows/alpha-release.yml`. The app is Developer ID signed, notarized and stapled, so it opens on any Apple Silicon Mac running macOS 26 without warnings. It also carries a minisign-signed update archive, so installed copies update themselves (ADR-0052, ADR-0063, ADR-0070).

## One-time setup (Account Holder)

The workflow fails before building when any of these repo secrets are missing.

1. **Developer ID Application certificate.**
   1. Go to developer.apple.com > Certificates > +, choose "Developer ID Application", and install it.
   2. In Keychain Access, export the certificate together with its private key as a `.p12`, with a password.
   3. Set the secrets:

      ```bash
      base64 -i DeveloperID.p12 | gh secret set DEVELOPER_ID_P12_BASE64 -R korbindeman/ochre
      ```

      ```bash
      gh secret set DEVELOPER_ID_P12_PASSWORD -R korbindeman/ochre
      ```

2. **Notarization key.**
   1. In App Store Connect, go to Users and Access > Integrations > Team Keys and create a key with the Developer role. Download the `.p8` file.
   2. Note the key ID and the issuer ID shown on that page.
   3. Set the secrets:

      ```bash
      gh secret set NOTARY_KEY -R korbindeman/ochre < AuthKey_XXXXXXXXXX.p8
      ```

      ```bash
      gh secret set NOTARY_KEY_ID -R korbindeman/ochre
      ```

      ```bash
      gh secret set NOTARY_ISSUER_ID -R korbindeman/ochre
      ```

3. **Update signing key.** `crates/core/src/update.rs` has the public key for minisign key `C8F78DF4A0195DA2` compiled in.
   - If you still have that secret key, set it:

     ```bash
     gh secret set UPDATE_SIGNING_PRIVATE_KEY -R korbindeman/ochre < minisign.key
     ```

     ```bash
     gh secret set UPDATE_SIGNING_PRIVATE_KEY_PASSWORD -R korbindeman/ochre
     ```

   - If it is lost, make a new pair with `minisign -G -p ochre.pub -s ochre.key`. Put `base64 < ochre.pub` into `PUBLIC_KEY_B64` in `update.rs`, commit it, and set the secrets as above. The July alphas published unsigned manifests and cannot auto-update anyway, so nobody loses an update path.

4. **Passkey entitlement (later).** When Apple grants `com.apple.developer.web-browser.public-key-credential`, create a Developer ID provisioning profile for the app ID, then set it:

   ```bash
   base64 -i Ochre.provisionprofile | gh secret set PROVISIONING_PROFILE_BASE64 -R korbindeman/ochre
   ```

   With the profile present, signing switches to the restricted entitlements. Without it, the app is signed without them and passkeys stay off (ADR-0061, ADR-0070).

## Cut a release

```bash
git tag v0.2.0-alpha.1 && git push origin v0.2.0-alpha.1
```

The tag sets the app version. The run builds, signs, notarizes, staples, checks that the update signature verifies against the key in `update.rs`, and publishes a pre-release with:

- `Ochre-<tag>-macos-arm64.zip`, which friends download
- `Ochre.app.tar.gz` and `latest.json`, which the updater reads

## Check it on a second Mac

1. Download the zip from the release page and drag `Ochre.app` to Applications. It must open with no "unidentified developer" or "damaged" dialog.
2. Cut the next tag. Within a launch or two, the installed copy shows the Update & Restart card. After the restart, Ochre > About shows the new version.

## Local checks

- `just build-release` signs with the first Developer ID identity in your keychain (`BROWSER_CODESIGN_IDENTITY` overrides). It stays ad-hoc signed when there is none.
- `scripts/notarize-macos-app.sh <App.app>` notarizes and staples a signed app. It uses `NOTARY_PROFILE` (from `xcrun notarytool store-credentials`) or the `NOTARY_*` API-key variables.
- The signing script refuses to finish if the host signature carries restricted entitlements without an embedded provisioning profile, because AMFI would kill that build at launch.
