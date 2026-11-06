# Releasing

How a version of 4x4 Remote is released. It is the owner's job and happens after the owner has used
the build that master holds. In the commands below, `X.Y.Z` stands for the version: a new major
number for a break of what [the changelog](../CHANGELOG.md) says the version number promises, a new
minor one for a new feature and a new patch one for a fix.

## Prepare

On a branch of its own, such as `release-X.Y.Z`, in this checkout:

1. Set the version: `version` under `[workspace.package]` in `Cargo.toml`, then
   `cargo update --workspace`, which brings the workspace crates in `Cargo.lock` to it and updates
   nothing else.
2. Edit `CHANGELOG.md`: date the version's section, whose heading is `## [1.0.0] - Unreleased` for
   the first release and `## [Unreleased]` after it, as `## [X.Y.Z] - YYYY-MM-DD`; open a new
   empty `## [Unreleased]` section above it; and add the link references at the end of the file,
   one for `Unreleased` and one for the version:

   ```markdown
   [Unreleased]: https://github.com/lexy898/4x4-remote/compare/vX.Y.Z...HEAD
   [X.Y.Z]: https://github.com/lexy898/4x4-remote/releases/tag/vX.Y.Z
   ```

   Only the first release has nothing to compare with: the reference of every later version is
   `.../compare/vA.B.C...vX.Y.Z`, where `vA.B.C` is the release before it.
3. Retake the screenshots if the UI changed since they were taken; see
   [Screenshots](development.md#screenshots).
4. Run `just check`.

## Publish

1. Open a pull request, wait for CI to be green and merge it locally, as the Git section of
   [`AGENTS.md`](../AGENTS.md) says (`git merge --no-ff`, then push `master`).
2. Tag the merge commit and push the tag. The tag is annotated, and the merge commit's `Cargo.toml`
   must already say `X.Y.Z`:

   ```bash
   git tag -a vX.Y.Z -m "4x4 Remote X.Y.Z"
   git push origin vX.Y.Z
   ```

3. Make the GitHub Release from the version's block of `CHANGELOG.md`: its lines between its
   heading and the next heading, without the link references. Look at the notes file before
   publishing:

   ```bash
   notes=$(mktemp)
   awk -v head="## [X.Y.Z]" '
       index($0, head) == 1 { on = 1; next }
       /^## \[/ || /^\[[^]]+\]: / { on = 0 }
       on
   ' CHANGELOG.md >"$notes"
   gh release create vX.Y.Z --title "4x4 Remote X.Y.Z" --notes-file "$notes"
   ```

## After the tag

GitHub's archive of a tag has a checksum only once the tag exists, so the package of the release
is added in the first commit after the tag, on a branch and a pull request as usual.

1. `packaging/arch/release/PKGBUILD` builds the package `4x4-remote` (`pkgname=4x4-remote`) from
   the tag's archive. It is the `packaging/arch/PKGBUILD` of `4x4-remote-git` with these
   differences:
   - `conflicts=(4x4-git)`, for the name of the `-git` package before 1.0, and no `provides`:
     `4x4-remote-git` already conflicts with `4x4-remote`, and pacman checks a conflict from either
     side;
   - `pkgver=X.Y.Z`, with no `pkgver()` function, and the source
     `$url/archive/refs/tags/v$pkgver.tar.gz`, renamed with a `$pkgname-$pkgver.tar.gz::` prefix.
     The archive unpacks into `4x4-remote-X.Y.Z`, which is where `prepare()`, `build()` and
     `package()` change to;
   - `sha256sums=('<the checksum>')` instead of `b2sums=('SKIP')`. Download the archive from that
     URL into an empty directory and take its checksum with `sha256sum`; never copy one from
     anywhere else.
2. The hook tests in `scripts/hooks/test-hooks.sh` cover it as they cover the `-git` file (what
   `makepkg --printsrcinfo` says, and the functions run by hand), plus one test that its `pkgver`
   equals the version in `Cargo.toml`, so that a later release cannot leave the file behind.
3. The Install section of `README.md` gets the line for the release package next to the `-git` one:
   how to build it from the file in `packaging/arch/release/`, and that it replaces
   `4x4-remote-git`. The bug report form asks for `pacman -Q` of either package.

## The AUR

The owner publishes both packages on the [AUR](https://aur.archlinux.org) from the owner's own
account; nothing in this repository does it. There, each package is a git repository of its own
with the PKGBUILD as it is here, its `.SRCINFO` and a `LICENSE` for these build files. Once the
packages are there, the Install section of `README.md` says so.

Once, before the first package:

1. Make an account on the AUR and tick Hide Email Address.
2. Make a key for it with `ssh-keygen -f ~/.ssh/aur`, paste `~/.ssh/aur.pub` into SSH Public Key
   under My Account, and add to `~/.ssh/config`:

   ```text
   Host aur.archlinux.org
     IdentityFile ~/.ssh/aur
     User aur
   ```

For each package, `4x4-remote-git` from `packaging/arch/PKGBUILD` and `4x4-remote` from
`packaging/arch/release/PKGBUILD`, in a directory outside this checkout (`checkout` is the path of
this checkout):

```bash
git -c init.defaultBranch=master clone ssh://aur@aur.archlinux.org/4x4-remote-git.git
cd 4x4-remote-git
git config user.name lexy898
git config user.email "$(git -C "$checkout" config user.email)"
cp "$checkout/packaging/arch/PKGBUILD" .
```

For `4x4-remote`, its name and its PKGBUILD take the place of the `-git` ones. The clone of a
package that does not exist yet is empty, and git warns so. The AUR shows the author of every
commit, and without the two `git config` lines git would sign with the global name and address
instead of the noreply one this checkout uses. Then:

1. Add `LICENSE`: the [0BSD](https://spdx.org/licenses/0BSD.html) text, which the AUR suggests for
   build files, with the copyright line of `LICENSE-MIT`.
2. Build and install the package from that directory once with `makepkg -si`, and start 4x4.
   makepkg writes the version it built into `pkgver=` of the `-git` PKGBUILD there, and leaves its
   build files next to it.
3. Only now make `.SRCINFO`, so that it names the same version: `makepkg --printsrcinfo >.SRCINFO`.
4. Commit only the three files and push: `git add PKGBUILD .SRCINFO LICENSE`, then
   `git commit -m "Initial import"` and `git push`.

Afterwards `4x4-remote` takes every release: copy the new release PKGBUILD, build it once, make
`.SRCINFO` again, commit as `Update to X.Y.Z` and push. `4x4-remote-git` takes a commit only when
its PKGBUILD changes; new commits on `master` alone need none, as the AUR asks of `-git`
packages.
