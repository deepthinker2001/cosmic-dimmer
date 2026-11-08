# Making a release

Publishing a GitHub release does most of the work: the
[Release workflow](.github/workflows/release.yml) builds the `.deb` from the
release's tag, attaches it with its checksum, and adds it to the signed apt
repository that GitHub Pages serves from the `gh-pages` branch. apt users get
the new version with their next system update.

## 1. Prepare the release on `main`

1. **Version.** Set the new version (for example `0.2.0`) in the root
   `Cargo.toml`, under `[workspace.package]`. Every crate in the workspace
   takes it from there. If `crates/dimmer-samsung` changed, bump its own
   `version` in its `Cargo.toml` too: it is a separate workspace.
2. **Lock file.** Run `cargo check --workspace` once so `Cargo.lock` records
   the new version. CI builds with `--locked` and fails if it doesn't.
3. **Changelog.** Add a `### 0.2.0 (date)` section at the top of the
   README's **Changelog**: what changed, from a user's point of view.
4. **Check.** Run `./scripts/check.sh`. It runs what CI runs.
5. **Commit and push**, then wait for the CI workflow on `main` to pass.

A version is never reused: once a version is published, a fix is a new
version (`0.2.1`), not a changed `0.2.0`. apt caches packages by version.

## 2. Publish the release

The tag must be `v` followed by the version in `Cargo.toml`, such as `v0.2.0`;
the workflow refuses a mismatch.

From a terminal, with the notes in a file (the version's changelog section
from the README works well, followed by the install steps from the previous
release's notes):

```sh
gh release create v0.2.0 --target main --title "Dimmer for COSMIC 0.2.0" --notes-file notes.md
```

Or on GitHub: **Releases → Draft a new release**, create the tag `v0.2.0` on
`main`, fill in the title and notes, and **Publish release**.

Don't attach a `.deb` yourself: the workflow builds and attaches it.

## 3. Check that it worked

1. **Actions → Release:** both jobs, `build` and `apt`, should pass. The run
   takes about 5 minutes.
2. **The release page** should list `cosmic-ext-dimmer_0.2.0-1_amd64.deb` and its
   `.sha256`.
3. **apt:** on a machine with the repository added, `sudo apt update` then
   `apt policy cosmic-ext-dimmer` should show the new version as the candidate.

## If the workflow fails

- **Rerun it for the tag**, after fixing the cause on `main`:

  ```sh
  gh workflow run release.yml -f tag=v0.2.0
  ```

  The build uses the source at the tag; the publishing script comes from
  `main`. If the fix has to be in the source, make a new version instead.

- **Publish by hand.** With the signing key's keyring on your machine and a
  checkout of the `gh-pages` branch:

  ```sh
  just deb
  GNUPGHOME=<keyring folder> scripts/publish-apt.sh <gh-pages checkout> \
      20B91D4AACA9BE9E19F48B8C4FC4764D9918AC0A target/debian/cosmic-ext-dimmer_0.2.0-1_amd64.deb
  ```

  Then commit and push the `gh-pages` checkout, and attach the same `.deb`
  and a `sha256sum` of it to the release.

## The signing key

The apt repository is signed with the key
`20B9 1D4A ACA9 BE9E 19F4 8B8C 4FC4 764D 9918 AC0A`
("deepthinker2001 (COSMIC Dimmer apt repository)", after the project's
earlier name). The workflow reads it from the repository secret
`APT_SIGNING_KEY`, and its fingerprint from the repository variable `APT_SIGNING_KEY_ID`. Its public half is served as
`key.gpg` on GitHub Pages.

**It expires on 2031-11-06.** Extend it well before then, or apt will refuse
the repository:

```sh
GNUPGHOME=<keyring folder> gpg --quick-set-expire 20B91D4AACA9BE9E19F48B8C4FC4764D9918AC0A 5y
GNUPGHOME=<keyring folder> gpg --armor --export-secret-keys 20B91D4AACA9BE9E19F48B8C4FC4764D9918AC0A \
  | gh secret set APT_SIGNING_KEY
```

Then rerun the workflow for the latest tag, which publishes the extended
public key as `key.gpg` and re-signs the repository. Users need to fetch the
new `key.gpg` again (the first install step in the README); nothing else
changes for them.

If the key is ever lost or exposed, make a new one, replace the secret and
the variable, publish, and tell users to fetch `key.gpg` again: the old key
can no longer be trusted, so apt would reject the repository until they do.
