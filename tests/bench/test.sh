#!/bin/sh
set -e

ANCHOR_BUILD_SBF_ARCH=v2 RUSTC_BOOTSTRAP=1 RUSTFLAGS="-Z emit-stack-sizes" anchor test --skip-lint
