# netfyr

Declarative network configuration for Linux, applied over netlink.

You describe the network state you want in YAML. netfyr reads the current state from
the kernel, works out the difference, and applies it over netlink. The query and apply
paths need no daemon.

Work here is driven spec by spec from
[netfyr/specs](https://github.com/netfyr/specs).

## Build and test

```
cargo build                    # compile the workspace
cargo test                     # unit tests
make test                      # shell integration tests
make test TAGS=ipv4,routing    # integration tests tagged ipv4 OR routing
make test TAGS=ipv4+routing    # integration tests tagged ipv4 AND routing
```

The workspace has no crates yet, so the cargo commands exit 101 until the first spec
lands one. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
