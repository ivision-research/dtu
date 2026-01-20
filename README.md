# dtu

`dtu` is a toolkit for testing entire Android devices, without requiring root access. The goal is to collect as much data as possible from a generic Android device and store it in formats that are accessible via the command line tool or associated library.

Check out the [release blog post](https://research.ivision.com/introducing-dtu.html).

## Installation

You can install `dtu` via `nix` or with `cargo`. The `nix` install will provide you with all of the associated binaries, of which there are quite a few, at known compatible versions if you use the `dtuEnv` package.

If you're installing with `cargo`, ensure you have the `sqlite` development libraries available and just do a simple `cargo install`.

After installation, run `dtu run-check` to see which required binaries you already have installed (if you used `nix` and `dtuEnv`, everything should already be available).

**Note that `dtu` currently is not developed with Windows support in mind.**

## Global Configuration

`dtu` allows for some configuration for all projects in the `~/.config/dtu/config.toml` file. This will let you specify your file store implementation which will be used for diffing `dtu` runs of different projects. There are two potential ways to specify this either [for S3](doc/example-global-config-s3.toml) or your [local file system](doc/example-global-config-local.toml).

Without this configured `dtu` won't be able to diff, which will be discussed later.

## Project Configuration

Every project _must_ specify the `DTU_PROJECT_HOME` environmental variable. This is not optional and `dtu` won't work without it. I highly recommend using `direnv` and setting this in a `.envrc` file at the base of your project directory and then forgetting about it. `dtu` can even do this for you with `dtu gen-envrc`.

Projects can be further configured with a file in `$DTU_PROJECT_HOME/config.toml`. This file is documented in [the example](doc/example-project-config.toml).

## Startup

The `tl;dr` for staring a project is:

```
# Pull all files -- this is a resource intensive operation and takes a while
dtu pull

# Analyze all smali files and populate the graph database. This is also a
# resource intensive operation
dtu graph setup

# Set up the sqlite database, this is used for a lot of queries throughout the program.
# You can pass `--no-diff`, but emulator diffing is highly recommended.
dtu db setup

# Set up the test application
dtu app setup

# Install the test application on the device
dtu app install
```

Note this series of commands takes a long time: you are pulling, decompiling, and analyzing the device's Java framework in the setup. The first three commands are required for anything else `dtu` does. The test application is not required for everything, but should be built and installed since the server is used for a lot of `dtu`s functionality.

### `dtu pull`

1. Discovery - uses `adb` and some shell commands to find all files of interest. This finds, among other things, `apk`, `apex`, and `jar` files containing framework and application code.
2. Pull - uses `adb pull` to pull the files off the device to a local file
3. Decompile - uses various tools to decompile files and convert them to `smali` for later analysis and reverse engineering


### `dtu graph setup`

Parse and analyze the decompiled `smali` files and create a graph database (sqlite) of the entire device's framework and every APK. Once this is complete, you will have an inheritance and call graph that can be queried. `dtu` provides some canned queries for this graph database, but it is often used behind the scenes even if you never query it directly. This database is also accessible via the `dtu` crate and Python bindings.

### `dtu db setup`

Set up the `sqlite` database. This collects a lot of data from the device, some of which is pulled via `adb` when this runs, and stores it for analysis or querying later. The database is saved in `dtu_out/sqlite/device.db` and is crucial for diffing devices. If `--no-diff` is not provided, this will use the configured file store to find the appropriate `device.db` for an emulator at the same API level.

### `dtu app setup`/`dtu app install`

This will create a test application that is installed on the device. This application gives itself _all normal level permissions_ (pulled out of the database just created) and runs a server that the `dtu` command line tool interacts with quite a bit for some functionality.

## Using dtu

After you're setup you can start actually using `dtu` for analysis. How you do this is up to you, but we'll mention some useful tips here.

### Diffing and `dtu diff ui`

Note: when you open the diff TUI via `dtu diff ui`, type `?` to get some basic help.

The `dtu diff` subset of commands work based on diffs with a given "diff source". The primary diff source is an emulator, but it can be any arbitrary device that has previously been analyzed by `dtu` and the `device.db` saved. Diffing is _very_ helpful and it is sometimes easiest to start your testing with `dtu diff ui` and poking at things that are not standard AOSP features. Diffing isn't perfect, but it's a great start. To set up an emulator diff, run all of the `dtu` setup steps against an emulator (use `--no-diff` for the db setup and `setup` instead of `full-setup` for the graph db) and store that `device.db` in your chosen file store implementation.

The diff UI also supports a few "hook" programs for quick analysis, to make use of them just make sure they're somewhere in your `PATH`:

- `dtu-open-file` program / `DTU_OPEN_EXECUTABLE` env var: When in the diff UI, you can often highlight something and hit `O` to invoke this program to open the associated file. This program is executed with two arguments: the absolute file path and a "search hint".
- `dtu-clipboard` program / `DTU_CLIPBOARD_EXECUTABLE` env var: If you have something highlighted and hit `c`, this will send a `dtu ...` command to interact with the highlighted item via the command line to this program on `stdin`.


The command line tool will look for emulator databases in: `~/.local/share/dtu/aosp/{API_LEVEL}/device.db` or your given file store implementations `aosp/{API_LEVEL}/device.db`.

### Looking around

Generally you're going to be interested in opening `smali` files for reverse engineering and `dtu` provides a pretty easy way to find the correct file for a given class: `dtu open-smali-file` (this has an alias of `dtu of` since it's fairly common to use). Personally I use a custom `vim` plugin that may be open sourced as well one day for translating the given `smali` file to Java using `smali` and `jadx` for easier reverse engineering.

### Interacting with targets

The `dtu` application provides a TCP server (`dtu app start-server` and `dtu app forward-server`) that allows you to interact with the device _as if you were the application_. This is very different from interacting with the device via `adb`, which is typically a more privileged context. You may see subtle differences between `adb shell service list` and `dtu sh service list` due to SELinux context differences for example. Some of the commands for interacting with the device are:

- `dtu sh` - Run a shell command
- `dtu provider` - Perform a lot of different operations against providers
- `dtu broadcast` - Send a broadcast
- `dtu start-activity` - Start an activity via an `Intent`
- `dtu start-service` - Start a service via an `Intent`
- `dtu call` - Invoke a method on a Service via its `IBinder`, this works for system services and application services

Look around `dtu --help` for more commands like these, as this documentation could get a bit stale at some point. Many of these commands can have their arguments pre-populated for you via `dtu-clipboard` in the `dtu diff ui` TUI.

### Using the test application

The test application is intended to be modified and rebuilt over and over to run various tests that require Kotlin code to execute instead of the limited functionality provided by the command line tool. There are templates for different tests that can be generated with the `dtu app new-*` commands, and tests can be run via the `dtu app run-test ...` command.

## File system Dumps

`dtu` can also work, somewhat hindered, based on file system dumps of Android devices. This is useful for cases where you don't have `adb` access to the device but can otherwise obtain a full copy if its root file system. To use this feature, check out [the example project configuration](doc/example-project-config.toml) and specifically the `device-access.dump` configuration and `can-adb` value.

There are a few limitations due to the static nature of this testing, but it has proven useful in the past.

## The dtu crate

`dtu` is a command line tool and a Rust crate. You can directly access the two databases, the test application server, and other potentially interesting features via this crate. The API should be stable across major releases. We try to maintain backwards compatibility when possible.

## The dtu Python module

`dtu` also [exposes some bindings via Python](./dtu-py). While not all functionality that is available in the Rust crate is available there is a significant amount:

- The `Context` object - `dtu.Context`
- Read only access to the graph - `dtu.GraphDB` (also` dtu.CachingGraphDB` to cache result as pickles)
- Read only access to the device database - `dtu.DeviceDB`
- ADB access - `dtu.Adb`
- Access to the application server - `dtu.AppServer`
- Device filesystem access - `dtu.DeviceFS`
- dtu file store access - `dtu.FileStore`

All custom types exported by the module are pickleable.

Documentation is available via `help(dtu)` in the Python REPL.
