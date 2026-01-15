# 5.0.0

This new version contains significant breaking changes to the graph database and also adds Python bindings.

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
- Create Python bindings for the databases and the application server
- Exposed some functionality that was not previous `pub` or was just `pub (crate)`
- Removed some restructions on `find-callers`

# 4.2.0

First public release :)
