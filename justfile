default: check

# The fast gates also run before each commit.
hooks:
    git config core.hooksPath tools/hooks

build:
    cargo build --workspace

# Full Rust gate: format, pedantic clippy, tests, supply-chain policy.
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo deny check --hide-inclusion-graph

# Android gate: ktlint with the Compose rules, and Android Lint (warnings as errors, existing findings in the baseline).
android-check:
    cd apps/android && ./gradlew ktlintCheck lintDebug

# Every release package into target/dist (see tools/release.sh): `just release`, or `just release deb rpm`.
release *kinds:
    tools/release.sh {{kinds}}

# "Andromeda Link.app" for macOS arm64, cross-built here: target/macos (see tools/macos/build.sh for its needs).
mac-app:
    tools/macos/build.sh

# A worktree that shares this checkout's target dir, so it does not rebuild every crate from scratch.
worktree name:
    git worktree add ../andromeda-link-{{name}} -b {{name}}
    ln -s {{justfile_directory()}}/target ../andromeda-link-{{name}}/target
