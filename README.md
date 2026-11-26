# portix-desktop

The Portix desktop app, written in Rust on [portix-ui](https://github.com/chimeramutate/portix-ui) (which sits on [portix-gpui](https://github.com/chimeramutate/portix-gpui)). It replaces the Flutter UI one feature at a time and reads the same `~/.portix/profiles.json`.

```sh
cargo run
```

Until portix-ui is published, it is a path dependency: clone it next to this repo (`../portix-ui`).
