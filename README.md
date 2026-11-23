# calc

`calc` is a powerful and accurate command line tool for evaluating arithmetic expressions and operating on lists of numbers. It provides several conveniences for use in a shell/terminal environment:

* Expressions are expected on the command line; basic usage is `calc 9+7`
* No quoting or escaping is needed. Alternatives are provided for characters that have special meaning to common shells such as `()*`
* Use ASCII and unicode characters to minimize on typing and construct natural looking expressions.
* Multiple styles of brackets to avoid escaping and aid readability of complex nesting.
* Opening brackets are optional. An unmatched closing bracket encapsulates all the numbers before it.
* Designed for pipelining. Receive expressions or numbers from stdin and apply partial expressions to them.
* Use a mix of infix notation `calc 1+2` and functional programming (spreadsheet) style `calc sum(1,2)` for access to scientific functions.

> **Note**: `calc` is mostly designed for one-shot calculations on the command line and the escalating complexity that sometimes follows. It does not provide an interactive shell of its own - `bc(1)` is your friend for that type of experience.

## Example usage

```sh
calc 0.1+0.2  →  0.3          # Accurate decimal arithmetic
calc {3x4}^2  →  144          # Shell-safe characters to avoid quoting/escaping
calc √[∑1…2³ + ∏3…4]  →  14.5 # Supports scientific characters
calc '(99 - {36÷3})%4'  →  3  # Nested brackets of different styles
calc 15+4+3}/2  →  11         # Unmatched closing bracket wraps everything prior
calc 2π12  →  75.39822        # Wieldy default precision for irrationals (see --precision)
calc avg 3,4,8  →  5          # Send lists to functions
seq 1 10 | calc sum  →  55    # Receive lists from stdin
date +%s | calc +60x60x24  →  # Apply partial expressions to values from stdin
```

## Constructing expressions

| Operators            | Meaning                                    |
|----------------------|--------------------------------------------|
| `+` , `-`            | add , subtract                             |
| `*` `x` `×` `·` `•`  | multiply                                   |
| `÷` `/`              | divide                                     |
| `%` `mod`            | modulo (remainder)                         |
| `^` , `²` , `³`      | power (right-associative) , square , cube  |

| Symbols              | Meaning                                    |
|----------------------|--------------------------------------------|
| `{}` `()` `[]`       | brackets, nestable                         |
| `…` `..` `...`       | ranges (stepping +1 or -1), e.g. `1…10`    |
| `π` `pi`             | π (f64 precision)                          |
| `@`                  | stdin, as a list                           |
| `°` `º`              | convert degrees to radians                 |

| Functions                     | Result                            |
|-------------------------------|-----------------------------------|
| `sqrt` `√`                    | square root                       |
| `sum` `∑` `Σ`                 | total (0 for an empty list)       |
| `product` `prod` `∏` `Π`      | product                           |
| `avg` `average` `mean` `x̄`    | arithmetic mean                   |
| `median` `min` `max`          |                                   |
| `mode`                        | most frequent (returns all ties)  |
| `stdev`                       | sample standard deviation (n − 1) |
| `abs` `floor` `ceil` `round`  | applied per list element          |
| `factorial` `fact` `!`        | per element; `fact(n)` = `n!`     |
| `ln` `log` `exp`              | per element; `log` is base 10     |
| `sin` `cos` `tan`             | per element; angles in radians    |
| `asin` `acos` `atan`          | per element; inverse, in radians  |
| `sinh` `cosh` `tanh`          | per element; hyperbolic           |

### Usage notes

- Adjacent terms multiply at the same precedence as `x`: `2π`, `3√2`, `6÷2{1+2}` = 9 .
- `%`/`mod` shares precedence with `x` and `÷`, and the result takes the sign of the divisor (floored, as in Python): `-7 mod 3` = 2, `7%-3` = -2. Non-integers work too: `5.5 mod 2` = `1.5` .
- `√` binds to the term right after it: `√4^2` is `(√4)^2`; write `√{4^2}` for the other reading.
- Superscripts are atomic and work left to right: `2²³` = `(2²)³` = `64` .
- `!` binds tightest of all, to the term just before it: `-3!` = -6, `2^3!` = 64, `√4!` = √24. Factorials are exact up to `33!`, and f64 up to `170!`.
- `°` binds like `!` and converts degrees to radians: `sin 30°` = 0.5. Divide by `1°` to convert back: `asin{0.5}/1°` = 30. The ordinal `º` (Option-0 on a Mac) is equivalent.
- Spaces are primarily for separating items in a list and can also be used to provide whitespace between operators. Take care with subtraction because `calc 1 -2` produces a list.
- Commas also separate elements and trailing commas are inert: `sum(1,2,3,)`.
- Operations are applied per element: `{1 2 3}x2` is 2 4 6, `2^{1 2 3}` is 2 4 8, `{1 2}+{10 20}` is 11 22. Lists between operators must be of the same length.
- Digit grouping on input can use `_` `'` `’` and non-breaking space characters: `calc √1_234_565.43` .
- Function names are case-insensitive, so spreadsheet habits work: `SUM(1,2,3)` .

### Shell notes

- `(` `)` and `*` typically need quoting or escaping; use the shell-safe equivalents instead: `{}` and `x` .
- `!` is safe at the end of a word (`calc 5!`, `calc 5! x2`) but anything that follows it
   triggers history expansion - use single quotes to prevent history expansion. `!!` is rejected because double factorial is not supported - use `{5!}!` for factorial-of-factorial.
- Avoid commas or `..` inside `{}` because brace expansion turns
  `avg{1,2,3}` into `avg1 avg2 avg3` and `{1..3}` into `1 2 3` before `calc` runs.
- zsh is particular about closing braces `}`:
  * When `{` and `}` are in different _words_ (separated by spaces) a parse error is thrown.
    For `calc 2x{3 + 4}` remove spaces or quote the whole thing.
  * When using an unmatched `}`, avoid a trailing space. `calc 2+4+8} /2` throws a parse error.

### Stdin notes

- With no expression arguments, stdin is evaluated and printed: each line can be any expression.
- Multiple lines of stdin are merged to form one list (or see `--rows` for alternate).
- When an expression only works element by element (no functions that combine or reorder), each result is printed as its line arrives, so `tail -f` pipelines work.
  A bad line is reported on stderr with its line number and the rest carry on; the exit status is 1.
- When the whole list is needed (`sum`, `avg`, …) a bad line means no result at all, since a total that silently skipped a line would be wrong.
- Values received from stdin can be combined with arguments to chain compound expressions.
  * Arguments that start with an operator (`+ x * ÷ / % ^ ² ³ ! mod`) continue the stdin list: `seq 3 | calc x2` .
  * So does a lone negative number: `calc 2x5 | calc -1` gives `9`. If stdin is empty, it is just the number.
  * Arguments that end with an operator or function apply stdin as their last operand: `seq 3 | calc 2^` .
  * An unmatched closing bracket also wraps the stdin value: `seq 1 3 | calc +4}/2` gives `2.5`, `3`, `3.5`.
  * `@` explicitly positions stdin, as often as needed: `seq 1 20 | calc max@-min@` .
  * Complete arguments don't read stdin, so `calc 1+2` is safe inside a `while read` loop.
    A lone negative number is the exception: in a loop, use `echo "$x" | calc` rather than `calc $x`.

## Output

Arithmetic is exact (i128 rationals) until a result is irrational or overflows, then it falls back to f64.

- Integers stay integers: `7/7` → `1`.
- Terminating decimals print in full: `1/64` → `0.015625`.
- Recurring and irrational results print up to 5 places (trailing zeros trimmed).
- f64 results of 1e16 or more print in scientific notation, since their low digits aren't real.

## Command line options

| Option syntax                     | |
|-----------------------------------|-|
| `-p` `--precision=N\|max`         | Set precision (decimal places) for output (default: `5`) |
| `--pretty[=always\|auto\|SAMPLE]` | Read and write numbers using the locale's style for digit grouping and decimals. |
| `-r` `--rows`                     | Preserve rows of input instead of merging into a single list |

Command line options must be specified first, before the expression, to avoid being mistaken for a negative number. `--` explicitly ends the options.

### Pretty numbers

With `--pretty`, the locale's number representation for digit grouping and decimals is detected and displayed on output. When stdin is present, the locale's style is used to interpret the values so that commas (`,`) are **not** treated as a list separator.

```shell
$ calc --pretty -p1 1000π                    # en_GB, en_US
3,141.6
$ calc --pretty -p1 1000π                    # de_DE
3.141,6
$ calc --pretty -p1 1000π | calc --pretty +1 # Not a list
3,142.6
$ calc --pretty=0,00,000.0 ∑1…1234}/2        # Provide sample style
3,80,997.5
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
