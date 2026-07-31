# Contributing to rustytune

Thanks for helping! A few ground rules keep the project's licensing
coherent — please read this before opening a pull request.

## License of contributions (inbound = outbound)

rustytune is licensed under **GPL-3.0-or-later with the App Store
additional permission** described in `LICENSE-EXCEPTION.md`. By
submitting a contribution you agree that it is licensed under those
same terms — the GPL *including* the additional permission. There is
no CLA and no copyright assignment; you keep your copyright.

Contributions offered under plain GPL without the additional
permission can't be merged, since they would strip the project's
ability to ship app-store builds.

## Developer Certificate of Origin

Every commit must be signed off (`git commit -s`), which adds a
`Signed-off-by:` line certifying the
[Developer Certificate of Origin](https://developercertificate.org/):
in short, that you wrote the change or otherwise have the right to
submit it under the project's license.

## Practical notes

- `make test` must pass (fmt, clippy `-D warnings`, all tests).
- `make bench` gives you a hardware-free Speeduino to test against.
- The web build embeds into the server binary — run `make web` before
  `cargo build` if the UI seems stale.
- `fixtures/speeduino202501_7.ini` comes from the Speeduino project
  and stays under its own license; don't mix project code into it.
