;; Starter manifest for running Rust commands in Guix.
;; Copy to a repository root as `manifest.scm` and adjust as needed.

(specifications->manifest
 (list "rust"
       "rust:cargo"
       "rust:tools"
       "gcc-toolchain"
       "nss-certs"))
