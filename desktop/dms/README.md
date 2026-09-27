# DankMaterialShell bridge

DeskUnlock does not store a prebuilt `dms-syauth` executable in Git.

The bridge is built from the MIT-licensed DankMaterialShell source pinned to:

`aa4b99def48637d86a69620c0a8f3cc6aa0c4092`

The build helper applies the small DeskUnlock authentication integration to
`quickshell/Modules/Lock/Pam.qml`, then runs the upstream `make build`.

The integration adds a dedicated `syauth-dms` PAM context and starts it shortly
after the lock screen reports that it is secured. A successful phone
authentication then follows DMS's normal primary-auth unlock path.

Run:

```sh
./build-dms-syauth.sh /path/to/output/dms-syauth
```

The package should install the resulting executable as `/usr/bin/dms-syauth`.
