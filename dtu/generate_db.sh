#!/bin/bash

setup_db() {
    local db_file
    local config_file
    db_file="$(pwd)/$1_sqlite.db"
    config_file="$(pwd)/$1_diesel.toml"
    if [ -f "$db_file" ]; then
        mv "$db_file" "$db_file.bkup"
    fi
    diesel --config-file "$config_file" --database-url "sqlite://$db_file" migration run
}

setup_db "device"
setup_db "meta"
setup_db "graph"
