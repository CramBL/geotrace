# Interactive, step-by-step release drivers. Each recipe walks the flow from
# RELEASES.md and prints every git/just command as `<command> ?` before running
# it, so you confirm each step yourself: Enter runs it, `s` skips it, `q` quits.
#
# All four assume you are on an up-to-date, clean `trunk`. They refuse to start
# otherwise. The prerelease recipes are fully linear (the `rc` tag sits on the
# release branch). The regular recipes open a PR and pause for you to merge it
# before tagging the merged commit.

TRUNK := "trunk"

# Release the GUI app: bump on a branch, open a PR, then tag the merged commit.
[group("release")]
app VERSION: (_flow "app" "release" VERSION)

# Cut a GUI app prerelease (e.g. 0.2.0-rc.1): a full dry run that publishes nothing.
[group("release")]
app-prerelease VERSION: (_flow "app" "prerelease" VERSION)

# Release the SDKs (Rust/Python/C/C++ in lockstep): bump on a branch, PR, then tag.
[group("release")]
sdk VERSION: (_flow "sdk" "release" VERSION)

# Cut an SDK prerelease (e.g. 0.2.0-rc.1): a full dry run that publishes nothing.
[group("release")]
sdk-prerelease VERSION: (_flow "sdk" "prerelease" VERSION)

[private]
_flow TRACK KIND VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    track="{{ TRACK }}"
    kind="{{ KIND }}"
    version="{{ VERSION }}"
    trunk="{{ TRUNK }}"

    case "$track" in
      app) bump="bump-app"; tag_prefix="app/v"; label="app" ;;
      sdk) bump="bump-sdk"; tag_prefix="geotrace-sdk-v"; label="sdk" ;;
      *) echo "error: unknown track '$track'" >&2; exit 1 ;;
    esac
    tag="${tag_prefix}${version}"
    branch="release/${label}-v${version}"
    msg="release ${label} v${version}"

    bold=$(tput bold 2>/dev/null || true)
    dim=$(tput dim 2>/dev/null || true)
    reset=$(tput sgr0 2>/dev/null || true)

    # Print a command, wait for confirmation, then run it (or skip / quit).
    ask() {
      printf '\n  %s%s%s ? ' "$bold" "$*" "$reset"
      local reply; read -r reply </dev/tty || reply=q
      case "$reply" in
        '') "$@" ;;
        s|S) printf '  %sskipped%s\n' "$dim" "$reset" ;;
        *) printf '  aborted\n'; exit 1 ;;
      esac
    }

    # Pause for a manual step (no command to run), then continue or quit.
    pause() {
      printf '\n  %s%s%s\n  [Enter] continue   [q] quit: ' "$bold" "$1" "$reset"
      local reply; read -r reply </dev/tty || reply=q
      [ -z "$reply" ] || { printf '  aborted\n'; exit 1; }
    }

    # Preflight: a clean trunk, level with origin. Bail with guidance otherwise.
    current=$(git rev-parse --abbrev-ref HEAD)
    [ "$current" = "$trunk" ] || { echo "error: on '$current', not '$trunk' — switch to $trunk first." >&2; exit 1; }
    if ! git diff --quiet || ! git diff --cached --quiet; then
      echo "error: working tree is dirty — commit or stash first." >&2; exit 1
    fi
    git fetch --quiet origin "$trunk"
    [ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/$trunk")" ] || {
      echo "error: local $trunk is not level with origin/$trunk — pull first." >&2; exit 1
    }

    printf '%sReleasing %s %s%s  (branch %s, tag %s)\n' "$bold" "$label" "$version" "$reset" "$branch" "$tag"
    printf '%sEnter runs the shown command, s skips it, q quits.%s\n' "$dim" "$reset"

    ask git switch -c "$branch"
    # The bump sets every version spot and promotes the changelog to $version.
    ask just qa::"$bump" "$version"
    # Match the Linux CI lockfile guard before opening the release PR.
    ask cargo update --workspace --locked

    # If the bump was a no-op (version set and changelog already promoted, e.g.
    # the first release where trunk already carries it) there is no PR to make:
    # tag trunk directly.
    if [ "$kind" = "release" ] && git diff --quiet; then
      echo
      echo "Version and changelog are already at $version, so there is nothing to bump or merge."
      echo "Tag $trunk directly instead:"
      echo "  git switch $trunk && git tag $tag && git push origin $tag"
      echo "Then delete this branch: git switch $trunk && git branch -D $branch"
      exit 0
    fi

    ask git commit -am "$msg"
    ask git push -u origin "$branch"

    if [ "$kind" = "prerelease" ]; then
      # The prerelease tag points at the branch commit. No PR or merge is involved.
      ask git tag "$tag"
      ask git push origin "$tag"
      echo
      echo "Prerelease $tag pushed. The release and smoke tests run in Actions (nothing is published)."
      echo "Watch them with: gh run list --limit 5"
      exit 0
    fi

    # Regular release: land the bump via PR, then tag the merged trunk commit.
    ask gh pr create --base "$trunk" --fill
    pause "Merge the PR once CI and review pass, then continue here."
    ask git switch "$trunk"
    ask git pull --ff-only

    # Confirm the bump landed on trunk before tagging. The guard checks the
    # version spots and the release's changelog section.
    if [ "$track" = "sdk" ]; then
      just qa::check-versions --expect "$version"
    else
      just qa::check-app --expect "$version"
    fi

    ask git tag "$tag"
    ask git push origin "$tag"
    echo
    echo "Release $tag pushed. The GitHub release, publishing, and smoke tests run automatically."
    echo "Watch them with: gh run list --limit 5"
