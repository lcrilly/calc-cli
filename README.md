# calc

`calc` is a powerful and accurate command line tool for evaluating arithmetic expressions and operating on lists of numbers. It provides several conveniences for use in a shell/terminal environment:

* Expressions are expected on the command line; basic usage is `calc 9+7`
* No quoting or escaping is needed. Alternatives are provided for characters that have special meaning to common shells such as `()*`
* Use ASCII and unicode characters to minimize on typing and construct natural looking expressions.
* Multiple styles of brackets avoid escaping aid readability for complex nesting.
* Opening brackets are optional. An unmatched closing bracket encapsulates all the numbers before it.
* Designed for pipelining. Receive expressions or numbers from stdin and apply partial expressions to them.
* Use a mix of infix notation `calc 1+2` and functional programming (spreadsheet) style `calc sum(1,2)` for access to scientific functions.

> **Note**: `calc` is mostly designed for one-shot calculations on the command line and the escalating complexity that sometimes follows. It does not provide an interactive shell of its own - `bc(1)` is your friend for that type of experience.

## Example usage

```sh
calc 0.1+0.2  →  0.3        # Accurate decimal arithmetic
calc {3x4}^2  →  144        # Shell-safe characters to avoid quoting/escaping
calc '√(99 - [36÷2])' → 9   # Nested brackets of different styles
calc 15+4+3}/2  →  11       # Unmatched closing bracket wraps everything prior
calc 2π12  →  75.39822      # Wieldy default precision for irrationals (see --precision)
calc avg 3,4,8  →  5        # Send lists to functions
seq 1 10 | calc sum  →  55  # Receive lists from stdin
date +%s | calc +60x60x24   # Apply partial expressions to values from stdin
```

## Constructing expressions

| Operators           | Meaning                                   |
|---------------------|-------------------------------------------|
| `+` , `-`           | add , subtract                            |
| `*` `x` `×` `·` `•` | multiply                                  |
| `÷` `/`             | divide                                    |
| `%` `mod`           | modulo (remainder)                        |
| `^` , `²` , `³`     | power (right-associative) , square , cube |

| Symbols             | Meaning                                   |
|---------------------|-------------------------------------------|
| `{}` `()` `[]`      | brackets, nestable                        |
| `…` `..` `...`      | ranges (stepping +1 or -1), e.g. `1…10`   |
| `√` `sqrt`          | square root                               |
| `π` `pi`            | π (f64 precision)                         |
| `@`                 | stdin, as a list                          |

| Functions                     | Result                               |
|-------------------------------|--------------------------------------|
| `sum` `∑` `Σ`                 | total (0 for an empty list)          |
| `product` `prod` `∏` `Π`      | product                              |
| `avg` `average` `mean` `x̄`    | arithmetic mean                      |
| `median` `min` `max`          |                                      |
| `stdev`                       | sample standard deviation (n − 1)    |
| `abs` `floor` `ceil` `round`  | applied per list element             |
| `ln` `log` `exp`              | per element; `log` is base 10        |
| `sin` `cos` `tan`             | per element; angles in radians       |

### Usage notes
- Adjacent terms multiply at the same precedence as `x`: `2π`, `3√2`, `6÷2{1+2}` = 9.
- `%`/`mod` shares precedence with `x` and `÷`, and the result takes the sign of the divisor (floored, as in Python): `-7 mod 3` = 2, `7%-3` = -2. Non-integers work too: `5.5 mod 2` = `1.5`.
- `√` binds to the term right after it: `√4^2` is `(√4)^2`; write `√{4^2}` for the other reading.
- Superscripts are atomic and work left to right: `2²³` = `(2²)³` = `64`
- Spaces are primarily for separating items in a list and can also be used to provide whitespace between operators. Take care with subtraction because `calc 1 -2` produces a list.
- Commas also separate elements and trailing commas are inert: `sum(1,2,3,)`.
- Operations are applied per element: `{1 2 3}x2` is 2 4 6, `2^{1 2 3}` is 2 4 8, `{1 2}+{10 20}` is 11 22. Lists between operators must be of the same length.
- Digit grouping on input can use `_` `'` `’` and non-breaking space characters: : `calc √1_234_565.43`
- Function names are case-insensitive, so spreadsheet habits work: `SUM(1,2,3)`.

### Shell notes

- `(` `)` and `*` need quoting or escaping, or use the shell-safe  equivalents: `{}` and `x` .
- zsh rejects a `}` that ends a word when its `{` is in a different word, so
  `calc 2x{3 + 4}` fails in zsh before `calc` runs. Keep each brace group free
  of spaces (`calc 2 x {3+4}`) or quote it. The same may apply to an
  unmatched `}` at the end of a word (`calc 2+4+8} /2`); keep it mid-word
  (`calc 2+4+8}/2`) or quote it. bash treats a lone `}` as literal.
- Avoid commas or `..` inside `{}` because brace expansion turns
  `avg{1,2,3}` into `avg1 avg2 avg3` and `{1..3}` into `1 2 3` before `calc` runs.
- Arguments and stdin must be UTF-8 so use the longform of `√`, `π` when needed.

### Stdin notes

- With no expression arguments, stdin is evaluated and printed: each line can be any expression.
- Multiple lines of stdin are merged to form one list (or see `--rows` for alternate).
- Values received from stdin can be combined with arguments to chain compound expressions.
  * Arguments that start with an operator (`+ x * ÷ / % ^ ² ³ mod`) continue the stdin list: `seq 3 | calc x2` .
  * Arguments that end with an operator or function apply stdin as their last operand: `seq 3 | calc 2^` .
  * An unmatched closing bracket also wraps the stdin value: `seq 1 3 | calc +4}/2` gives `2.5`, `3`, `3.5`.
  * `@` explicitly positions stdin, as often as needed: `seq 1 20 | calc max@-min@` .
  * Complete arguments don't read stdin, so `calc 1+2` is safe inside a `while read` loop.
- When an expression only works element by element (no functions that combine or reorder), each result is printed as its line arrives, so `tail -f` pipelines work.
  A bad line is reported on stderr with its line number and the rest carry on; the exit status is 1.
- When the whole list is needed (`sum`, `avg`, …) a bad line means no result at all, since a total that silently skipped a line would be wrong.

## Output

Arithmetic is exact (i128 rationals) until a result is irrational or overflows, then it falls back to f64.

- Integers stay integers: `7/7` → `1`.
- Terminating decimals print in full: `1/64` → `0.015625`.
- Recurring and irrational results print up to 5 places (trailing zeros trimmed).
- A decimal anywhere in the input keeps a decimal in the output: `2.5x2` → `5.0`.
- f64 results of 1e16 or more print in scientific notation, since their low digits aren't real.

## Command line options

| Option syntax                     | |
|-----------------------------------|-|
| `-p` `--precision=N\|max`         | Set precision (decimal places) for output (default: `5`) |
| `--pretty[=always\|auto\|SAMPLE]` | Read and write numbers using the locale's style, including digit grouping. `SAMPLE` explicitly sets the style, e.g. `10,00,000.1` |
| `-r` `--rows`                     | Preserve rows of input instead of merging into a single list |

Command line options must be specified first, before the expression, to avoid being mistaken for a negative number. `--` explicitly ends the options.

### Pretty numbers

Plain input is the same everywhere: `.` for decimals, `,` or spaces between list elements, `_` to group digits. So a script gives the same answer on every machine.
`--pretty` switches to the style people read:

```shell
$ calc --pretty -p1 1000π                    # en_GB, en_US
3,141.6
$ calc --pretty -p1 1000π                    # de_DE
3.141,6
$ calc --pretty -p1 1000π | calc --pretty +1 # --pretty reads the style back
3,142.6
$ calc --pretty=1,23,456.7 12345678          # or give an example of the style
1,23,45,678
```

- `--pretty=auto` only applies to a terminal, so pipes get plain numbers; `--pretty=never` turns it off.
- Under `--pretty`, numbers are read with the local decimal mark and grouping. A group separator only counts between correctly sized groups, so `1,2` is still the list 1, 2.
- `CALC_PRETTY=auto` (or `always`/`never`) sets the default for output. It never changes how input is read.
  With `CALC_PRETTY=auto` in your shell profile, the terminal shows `3,141.6` while `calc 1000π | calc +1` still works.
- The style comes from, in order: `--pretty=SAMPLE`, `CALC_NUMBER_FORMAT=SAMPLE`, `LC_ALL`/`LC_NUMERIC` if set, macOS System Settings → Language & Region (including custom number formats), then `LANG`.
- In a locale that writes decimals with a comma, an input like `3,5` is an error rather than a list; write `3.5`, or `3, 5` for the list.

## Build & install

`calc` was written in Rust by claude-opus-5-5. Install the Rust toolchain from your preferred package manager or get it from https://rustup.rs/

```
cargo build --release      # → target/release/calc
cargo install --path .     # → ~/.cargo/bin/calc
```
