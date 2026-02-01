# Application Notes

Hands-on demonstrations of sysprims features. Each app note is a self-contained scenario you can run locally to understand a capability.

## Index

| App Note | Feature | Description |
|----------|---------|-------------|
| [multi-pid-kill](multi-pid-kill/) | `sysprims kill` | Batch signal delivery vs tree termination |

## Running App Notes

App notes are designed to be self-contained:

1. Build sysprims: `make build`
2. Navigate to the app note: `cd docs/appnotes/<name>/`
3. Follow the README instructions

All scripts use POSIX shell - no additional toolchains required.

## Contributing

When adding a new app note:

1. Create a slug-named folder: `docs/appnotes/<feature-name>/`
2. Include a `README.md` with clear steps
3. Keep scripts portable (POSIX sh, no bash-isms)
4. Clean up after yourself (kill spawned processes)
5. Add an entry to this index
