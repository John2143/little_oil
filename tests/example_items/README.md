# Example items (tooltip corpus)

Drop pasted item tooltips here as `.txt` files — the test
`every_example_item_parses` picks up **every** `.txt` file in this directory
recursively, no registration needed. You can paste thousands.

## Workflow

1. In Path of Exile, hover the item and press `Ctrl+C` to copy its tooltip.
2. Save it as a `.txt` file anywhere under this directory — a subdirectory is
   fine (organize by league, rarity, item class, or crafting system).
3. Run the corpus:

   ```sh
   nix develop --command cargo test every_example_item_parses
   ```

   Any file that the parser cannot handle fails the test, and the failure
   message names each failing file. Fix the parser (or add the paste to
   `broken/` below) until it passes.

## Rules

- Everything under this directory must parse — that is the point: real
  tooltips are the regression suite.
- A file that is **known-unparseable** (corrupted paste, unsupported line
  shape) goes in a `broken/` subdirectory at any depth. `broken/` is skipped.
- Non-`.txt` files (like this README) are ignored.
