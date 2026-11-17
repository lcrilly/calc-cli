# calc

`calc` is a powerful and convenient command line tool for evaluating arithmetic expressions and lists of numbers in a shell/terminal environment. Notably, alternatives are provided for characters that typically require escaping or quoting.

```shell
$ calc 2x{3+4}                     # Easy bracketing
14
$ calc {{2+3}x{4-1}}÷5             # Nesting
3
$ calc 2+4+8}/2                    # Unmatched closing bracket wraps everything before it
7
$ calc 1/3                         # Default precision
0.33333
$ calc -p=10 √2                    # Configurable precision
1.4142135624
$ echo 2π | calc --precision=max   # Receive expressions from stdin (and per-architecture precision)
6.283185307179586
$ seq 1 10 | calc 2^ | tr '\n' ,   # Compound expressions that combine stdin and command line arguments
2,4,8,16,32,64,128,256,512,1024,
$ seq 1 100 | calc sum             # Lines of stdin are a list
5050
$ calc avg 3 4 8                   # Functions take lists
5
$ calc --pretty -p1 1000π          # Local number style (here en_GB)
3,141.6
```

## Constructing expressions

| Symbol             | Meaning                                  |
|--------------------|------------------------------------------|
| `+` , `-`          | add , subtract                           |
| `x` `*` `×` `·` `•`| multiply (also `⋅` and `∙`)              |
| `÷` `/`            | divide                                   |
| `%` `mod`          | modulo (remainder)                       |
| `^`                | power (right-associative)                |
| `{}` `()` `[]`     | brackets, nestable                       |
| `√` `sqrt`         | square root                              |
| `π` `pi`           | π (f64 precision)                        |
| `1 2 3` `1,2,3`    | a list                                   |
| `1..10` `1…10`     | a range (step 1, either direction)       |
| `@`                | stdin, as a list                         |

- Adjacent terms multiply at the same precedence as `x`: `2π`, `3√2`, `6÷2{1+2}` = 9.
- `%`/`mod` shares precedence with `x` and `÷`, and the result takes the sign of the divisor (floored, as in Python): `-7 mod 3` = 2, `7%-3` = -2. Non-integers work too: `5.5 mod 2` = `1.5`.
- A closing bracket with no opening bracket implies one at the very start, so `2+4+8}/2` is `{2+4+8}/2` = 7.
  It wraps the whole of what comes before, not just the last term: `1+2x3}` = 7, `-2}^2` = 4.
  Repeat it to nest outwards: `1+2}x3}+1` is `{{1+2}x3}+1` = 10.
  A closer that doesn't match a bracket that is already open (`{1+2)`) is still an error.
- `√` binds to the term right after it: `√4^2` is `(√4)^2`; write `√{4^2}` for the other reading.

### Lists and spaces

Every value is a list; a single number is a list of one. Arguments are joined with spaces.

- A space between two numbers (or bracketed groups, constants, functions) separates list elements: `calc 1 2 3`, `calc 2 π`.
  Next to an operator it is ignored, so `calc 2 x {3 + 4}` is 14.
- A sign attached to a number after a space starts a new element: `1 -2` is the list 1, −2, while `1 - 2` and `1-2` are −1.
- Commas separate elements too, and a trailing comma is allowed: `sum(1,2,3,)`.
- Arithmetic applies element by element: `{1 2 3}x2` is 2 4 6, `2^{1 2 3}` is 2 4 8, `{1 2}+{10 20}` is 11 22.
  Lists of different lengths (other than a single number) are an error.
- A list result prints one number per line.
- Group digits with `_`: `1_000_000`. `'`, `’` and no-break spaces between digits work too.
  `1,234` and `1 000` are errors rather than quietly becoming lists; write `1, 234` if you mean the list.

### Functions

| Function                                | Result                               |
|-----------------------------------------|--------------------------------------|
| `sum` `∑` `Σ`                           | total (0 for an empty list)          |
| `product` `prod` `∏` `Π`                | product                              |
| `count`                                 | number of elements                   |
| `avg` `mean` `average` `x̄`              | arithmetic mean                      |
| `median` `min` `max`                    |                                      |
| `stdev`                                 | sample standard deviation (n − 1)    |
| `cumsum` `cumprod`                      | running total / product              |
| `sort`                                  | ascending order                      |
| `abs` `floor` `ceil` `round`            | per element; `round` is half away from zero |
| `ln` `log` `exp` `sin` `cos` `tan`      | per element; `log` is base 10, angles in radians |

- Names are case-insensitive, so spreadsheet habits work: `SUM(1,2,3)`.
- `sum{1 2 3}` and `sum(1,2,3)` take the bracket; `sum 1 2 3` takes everything after it, up to the end of the enclosing bracket.
  With an unmatched closer that gives `calc sum 1 2 3}x2` = 12.
- `x̄` is `x` followed by a combining macron (U+0304) or overline (U+0305).
- On a Mac, `•` is Option-8, `·` is Option-Shift-9, `∑` is Option-W, `∏` is Option-Shift-P and `…` is Option-; (`...` works too).

## Stdin

- With no expression arguments, stdin is evaluated and printed: each line can be any expression.
- The lines of stdin form one list (a line containing a list adds all of its elements): `seq 10 | calc sum` is 55.
- Values received from stdin can be combined with arguments to chain compound expressions.
  - Arguments that start with an operator (`+ x * × ÷ / % ^ mod`) continue the stdin list: `seq 3 | calc x2`.
  - Arguments that end with an operator or function apply stdin as their last operand: `seq 3 | calc 2^`, `seq 10 | calc avg`.
  - An unmatched closing bracket wraps the stdin value too: `seq 1 3 | calc +4}/2` gives `2.5`, `3`, `3.5`.
  - `@` places stdin explicitly, as often as needed: `seq 10 | calc 'sum{@}/count{@}'`.
  - Complete arguments don't read stdin, so `calc 1+2` is safe inside a `while read` loop.
- `-r`/`--rows` preserves rows instead: each line is its own list and gives one line of output, with its results separated by spaces.
  `printf '1 2\n3 4\n' | calc -r sum` gives `3`, `7` where `calc sum` gives `10`, and `calc -r cumsum` restarts the running total on each line.
  A bad line is reported and the other lines still give results.
- When an expression only works element by element (no functions that combine or reorder), each result is printed as its line arrives, so `tail -f` pipelines work.
  A bad line is reported on stderr with its line number and the rest carry on; the exit status is 1.
- When the whole list is needed (`sum`, `sort`, …) a bad line means no result at all, since a total that silently skipped a line would be wrong.

## Output

Arithmetic is exact (i128 rationals) until a result is irrational or overflows, then it falls back to f64.

- Integers stay integers: `7/7` → `1`.
- Terminating decimals print in full: `1/64` → `0.015625`.
- Recurring and irrational results print up to 5 places (trailing zeros trimmed).
- A decimal anywhere in the input keeps a decimal in the output: `2.5x2` → `5.0`.
- f64 results of 1e16 or more print in scientific notation, since their low digits aren't real.

## Options

Options come before the expression; `--` ends them.

- `-p`/`--precision`: `--precision=N`, `-p=N`, `-pN` or `-p N`, where `N` is a number of decimal places or `max` (full f64 precision, shortest round-trip form).
- `-r`/`--rows`: preserve rows, evaluating each line of stdin as its own list.
- `--pretty[=always|auto|EXAMPLE]`: write numbers in the local style, and read them that way too.

### Local number style

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

- `--pretty` is `--pretty=always`; `--pretty=auto` only applies to a terminal, so pipes get plain numbers; `--pretty=never` turns it off.
- Under `--pretty`, numbers are read with the local decimal mark and grouping. A group separator only counts between correctly sized groups, so `1,2` is still the list 1, 2.
- `CALC_PRETTY=auto` (or `always`/`never`) sets the default for output. It never changes how input is read.
  With `CALC_PRETTY=auto` in your shell profile, the terminal shows `3,141.6` while `calc 1000π | calc +1` still works.
- The style comes from, in order: `--pretty=EXAMPLE`, `CALC_NUMBER_FORMAT=EXAMPLE`, `LC_ALL`/`LC_NUMERIC` if set, macOS System Settings → Language & Region (including custom number formats), then `LANG`.
- In a locale that writes decimals with a comma, an input like `3,5` is an error rather than a list; write `3.5`, or `3, 5` for the list.

## Shell notes

- zsh rejects a `}` that ends a word when its `{` is in a different word, so
  `calc 2x{3 + 4}` fails in zsh before `calc` runs. Keep each brace group free
  of spaces (`calc 2 x {3+4}`) or quote it. The same may apply to an
  unmatched `}` at the end of a word (`calc 2+4+8} /2`); keep it mid-word
  (`calc 2+4+8}/2`) or quote it. bash treats a lone `}` as literal.
- `(` `)` and `*` still need quoting, or use the `{}` and `x` equivalents.
- `..` ranges, `@`, `,` and `_` need no quoting in bash or zsh.
- Don't put commas or `..` directly inside `{}`: the shell's brace expansion turns
  `avg{1,2,3}` into `avg1 avg2 avg3` and `{1..3}` into `1 2 3` before `calc` runs.
  Write `avg 1,2,3`, `avg{1 2 3}` or `avg{1, 2, 3}` instead (a space stops the expansion).
  `calc` refuses the expanded form of a function call rather than give a wrong answer,
  but it can't spot every case: `{1,2}+{10,20}` becomes four separate sums.
- Arguments and stdin must be UTF-8; in a non-UTF-8 locale
  `calc` reports an error rather than misreading `÷`, `√` or `π`.
   The ASCII forms (`/`, `sqrt`, `pi`) work in any locale.

## Build & install

`calc` was written in Rust by claude-opus-5-5. Install the Rust toolchain from your preferred package manager or get it from https://rustup.rs/

```
cargo build --release      # → target/release/calc
cargo install --path .     # → ~/.cargo/bin/calc
```
