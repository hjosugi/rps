# Install

## Build from source

`rpsh` is a Rust workspace. The checked-in toolchain is the version CI uses.

```console
$ git clone https://github.com/hjosugi/rpsh.git
$ cd rpsh
$ cargo build --release
```

The executable is `target/release/rpsh`:

```console
$ target/release/rpsh -Version
rpsh <version> (targets latest stable PowerShell)
```

To put it on `PATH`, copy it somewhere already on `PATH`, or:

```console
$ export PATH="$PWD/target/release:$PATH"
```

## Optional: reference pwsh

The conformance cases are compared with **recorded** output, so running the
suite needs no `pwsh`. Recording or updating cases does:

```console
$ scripts/fetch-pwsh.sh        # latest stable pwsh into .cache/
$ cargo harness conform record --filter <case>
```

## Documentation

The site is built with MkDocs Material:

```console
$ python3 -m venv .venv
$ . .venv/bin/activate
$ pip install -r requirements-docs.txt
$ mkdocs serve                 # http://127.0.0.1:8000
```

To build the static site:

```console
$ mkdocs build                 # site/
```

Pushing to `main` publishes the site through the `docs` GitHub Actions
workflow.

## Platform support

CI builds and tests Linux x64, Linux arm64 and macOS arm64. The reference
conformance goldens are recorded on Linux, which is the primary target. Output
that depends on a platform is part of the conformance contract, not a best
effort.
