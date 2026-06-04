# 6.0.0

- **BREAKING** No longer output `strings.txt` with smalisa output. This has been reworked to include strings in the graph database so they can be queried alongside the graph. It is still possible to get all strings for a given source, but this file no longer exists
- **BREAKING** Updated the GraphDatabase trait to include string and field related methods
- **BREAKING** Update to apktool v3.0.2 -- this is breaking because the new apktool doesn't allow specifying `-api`
- **BREAKING** Changed the way `list system-services/system-service-methods/apks` works to take a diff source similar to other commands
- **BREAKING** Removed redundant commands under `diff` that were covered (better) by commands under `list`
-- **BREAKING** Tried to make some of the flags more consistent across calls, `-n/--only-new` vs `-N/--only-new` was a big one.
- Allow specifying graph sources by APK instead of squashed paths
- Major bugfixes:
    - Permissions were UNIQUE(name), updated to UNIQUE(name, source_apk_id)
    - Missing index on a graph query made it take very long
- Surface method IDs in graph queries
- Updated smali/baksmali/jadx versions in the nix flake
- Add `DTU_DIFF_SOURCE` env var for specifying a default diff source
- Added `find strings` to find strings by various criteria
- Added `find methods` to find methods by various criteria
- Added `find fields` to find fields by various criteria
- Added `meta` to the `dtu list IPC` command output JSON
- Added a few more `-j/--json`s
- Started adding a hidden `_scripting`/`_s` command that does some metadata related tasks for making shell scripts easier to write
- Test app look updated

# 5.0.0

This new version contains significant breaking changes to the graph database and also adds Python bindings.

- **BREAKING** Flattened the `db` module structure
- **BREAKING** Changed graph database backend to sqlite instead of cozo
- **BREAKING** Changed the `GraphDatabase` trait significantly
    - Removed the REPL functionality -- this is straightforward for sqlite
    - Changed the way method search works by adding more methods
- **BREAKING** Completely changed the way the graph database is setup
    - Removed the concept of partial versus full setup
    - Moved more functionality for the initial setup into the `GraphDatabaseSetup` trait
- **BREAKING** Moved some CLI commands around to try to create a more consistent interface. `graph canned` is gone and all commands now exist under `find`. Some `find` subcommands moved to `list`.
- **BREAKING** Changed the way prerequisistes are stored in the meta database
- **BREAKING** Made `FileStore` require `Send` and `Sync`
- **BREAKING** `meta set-progress` now sets to true and `clear-progress` added to set to false
- **BREAKING** Completely removed the `DeviceDatabase` trait. This database will always be `sqlite` and it just added complexity and actually make things less efficient. `DeviceSqliteDatabase` was renamed to `DeviceDatabase`
- **BREAKING** Reworked configuration
- Exposed the `SqliteConnection` on the `DeviceDatabase` and `SqliteGraphDatabase` for direct access
- Added some methods to `DeviceDatabase`
- Create Python bindings for the databases and the application server
- Exposed some functionality that was not previous `pub` or was just `pub (crate)`
- Removed some restructions on `find-callers`
- Added `dtu-complete`
- Added the ability to call methods on Intent's via command line intents with `#m:`
- Added `L` to the diff UI for logcat strings
- Added `Message` types to the Parcel mini language
- Added `find manifest` for APK manifest files
- Added `find outgoing-calls` for outgoing calls
- Added `find class-with-method` to find classes that define a given method

# 4.2.0

First public release :)
