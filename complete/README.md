# dtu-complete

This provides an easy way to get detailed completions from your shell. For example, `dtu call system-service -s <TAB>` will complete the `<SERVICE>` from the database if available. To incorporate this with your own shell:

- Set `DTUC_SHELL` to a flavor that is similar to your own. If nothing is similar, you'll need to update this program and make a pull request
- Set `DTUC_INCOMPLETE` to the word that is currently being completed
- Call `dtu-complete -- ALL ARGS BEFORE DTUC_INCOMPLETE` -- do _not_ include `DTUC_COMPLETE` with the args.

See the [fish bindings](./shell/dtu.fish) for an example.
