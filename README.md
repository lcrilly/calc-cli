# calc

`calc` is a powerful and convenient command line tool for evaluating arithmetic expressions in a shell/terminal environment. Notably, alternatives are provided for characters that typically require escaping or quoting.

```shell
$ calc 2x{3+4}
14
$ calc {{2+3}x{4-1}}÷5
3
$ calc 1/3
0.33333
$ calc -p=10 √2
1.4142135624
$ echo 2π | calc --precision=max
6.283185307179586
```

## Constructing expressions

| Symbol      | Meaning                   |
|-------------|---------------------------|
| `+` , `-`   | add , subtract             |
| `x` `*` `×` | multiply                  |
| `÷` `/`     | divide                    |
| `^`         | power (right-associative) |
| `{}` `()`   | brackets, nestable        |
| `√` `sqrt`  | square root               |
| `π` `pi`    | π (f64 precision)         |

- Spaces are ignored and multiple arguments are joined, so `calc 1 000 x 3` is `3000`.
- Adjacent terms multiply at the same precedence as `x`: `2π`, `3√2`, `6÷2{1+2}` = 9.
- `√` binds to the term right after it: `√4^2` is `(√4)^2`; write `√{4^2}` for the other reading.
- With no expression arguments each line of stdin is evaluated; a failing line goes to stderr and the exit status is 1.

## Output

Arithmetic is exact (i128 rationals) until a result is irrational or overflows, then it falls back to f64.

- Integers stay integers: `7/7` → `1`.
- Terminating decimals print in full: `1/64` → `0.015625`.
- Recurring and irrational results print up to 5 places (trailing zeros trimmed).
- A decimal anywhere in the input keeps a decimal in the output: `2.5x2` → `5.0`.
- f64 results of 1e16 or more print in scientific notation, since their low digits aren't real.

`-p`/`--precision` must be the first argument: `--precision=N`, `-p=N`, `-pN`
or `-p N`, where `N` is a number of decimal places or `max` (full f64 precision,
shortest round-trip form).

## Shell notes

- zsh rejects a `}` that ends a word when its `{` is in a different word, so
  `calc 2x{3 + 4}` fails in zsh before `calc` runs. Keep each brace group free
  of spaces (`calc 2 x {3+4}`) or quote it.
- `(` `)` and `*` still need quoting, or use the `{}` and `x` equivalents.
- Arguments and stdin must be UTF-8; in a non-UTF-8 locale
  `calc` reports an error rather than misreading `÷`, `√` or `π`.
   The ASCII forms (`/`, `sqrt`, `pi`) work in any locale.

## Build & install

`calc` was written in Rust by claude-opus-5-5. Install the Rust toolchain from your preferred package manager or get it from https://rustup.rs/

```
cargo build --release      # → target/release/calc
cargo install --path .     # → ~/.cargo/bin/calc
```
