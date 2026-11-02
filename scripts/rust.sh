#!/bin/sh
set -eu
export CARGO_HOME="$PWD/.guix-shell/cargo"
exec guix shell -m manifest.scm -CWNF --preserve='^CARGO_HOME$' -- "$@"
