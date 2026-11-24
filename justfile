# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later

fmt:
    nix fmt

build:
    cargo build --locked --workspace

clippy:
    cargo clippy --locked --workspace --all-targets -- --deny warnings

test:
    cargo test --locked --workspace

doc:
    RUSTDOCFLAGS="--deny warnings" cargo doc --locked --workspace --no-deps

deny:
    cargo deny check bans licenses sources

audit:
    cargo audit

reuse:
    reuse lint

vm-test:
    nix build --no-link --print-build-logs .#checks.x86_64-linux.vm-pipeline .#checks.x86_64-linux.vm-gnome-extension

vm-test-pipeline:
    nix build --no-link --print-build-logs .#checks.x86_64-linux.vm-pipeline

vm-test-gnome:
    nix build --no-link --print-build-logs .#checks.x86_64-linux.vm-gnome-extension

check: clippy test doc deny audit reuse
    nix flake check
