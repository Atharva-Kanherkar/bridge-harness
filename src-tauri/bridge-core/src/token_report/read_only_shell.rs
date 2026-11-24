use super::{is_assignment, is_number, is_sed_print, program_name, wrapped_script, SHELL_OPERATORS};
use serde_json::Value;

const FORBIDDEN: [char; 10] = ['<', '>', '$', '`', '(', ')', '\\', '\r', '\n', '\0'];

pub(super) fn matches(command: &Value) -> bool {
    match command {
        Value::String(text) => read_text(text, true),
        Value::Array(items) => {
            let Some(words) = items.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else {
                return false;
            };
            if words.first().is_some_and(|word| is_assignment(word))
                || words.iter().any(|word| has_forbidden_chars(word))
            {
                return false;
            }
            match wrapped_script(&words) {
                Some(script) => read_text(script, false),
                None => {
                    !words.iter().any(|word| word.contains(&SHELL_OPERATORS[..])) && source(&words)
                }
            }
        }
        _ => false,
    }
}

fn has_forbidden_chars(text: &str) -> bool {
    text.contains(&FORBIDDEN[..])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Separator {
    And,
    Chain,
    Pipe,
    End,
}

// Each separator belongs to the stage before it.
fn stages(text: &str) -> Option<Vec<(&str, Separator)>> {
    if has_forbidden_chars(text) {
        return None;
    }
    let mut stages = Vec::new();
    let mut quote = None;
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
            continue;
        }
        let (separator, width) = match ch {
            '\'' | '"' => {
                quote = Some(ch);
                continue;
            }
            '&' if chars.next_if(|&(_, next)| next == '&').is_some() => (Separator::And, 2),
            ';' => (Separator::Chain, 1),
            '|' => (Separator::Pipe, 1),
            '&' | '*' | '?' | '[' | ']' | '{' | '}' | '~' | '#' | '!' => return None,
            _ => continue,
        };
        stages.push((&text[start..index], separator));
        start = index + width;
    }
    if quote.is_some() {
        return None;
    }
    stages.push((&text[start..], Separator::End));
    Some(stages)
}

fn read_text(text: &str, unwrap: bool) -> bool {
    let Some(stages) = stages(text) else {
        return false;
    };
    let mut after_pipe = false;
    for (index, (stage, separator)) in stages.into_iter().enumerate() {
        let Ok(owned) = shell_words::split(stage) else {
            return false;
        };
        let words: Vec<_> = owned.iter().map(String::as_str).collect();
        if words.first().is_some_and(|word| is_assignment(word)) {
            return false;
        }
        if unwrap && index == 0 && separator == Separator::End {
            if let Some(script) = wrapped_script(&words) {
                return read_text(script, false);
            }
        }
        if let ["cd", directory] = words.as_slice() {
            if index == 0
                && separator == Separator::And
                && literal(directory)
                && !directory.strip_prefix('+').is_some_and(is_number)
            {
                continue;
            }
        }
        let valid = if after_pipe {
            filter(&words)
        } else {
            source(&words)
        };
        if !valid {
            return false;
        }
        after_pipe = separator == Separator::Pipe;
    }
    true
}

fn literal(word: &str) -> bool {
    !word.is_empty() && !word.starts_with('-')
}

type ValueOption = (&'static str, fn(&str) -> bool);

struct Arguments<'a> {
    operands: Vec<&'a str>,
    options: Vec<&'a str>,
}

fn arguments<'a>(
    args: &[&'a str],
    short_flags: &str,
    flags: &[&str],
    value_options: &[ValueOption],
) -> Option<Arguments<'a>> {
    let mut operands = Vec::new();
    let mut options = Vec::new();
    let mut args = args.iter().copied();
    while let Some(arg) = args.next() {
        if arg == "--" {
            operands.extend(args);
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            operands.push(arg);
            continue;
        }
        let (name, attached) = if arg.starts_with("--") {
            arg.split_once('=')
                .map_or((arg, None), |(name, value)| (name, Some(value)))
        } else {
            arg.split_at_checked(2)
                .map_or((arg, None), |(name, value)| {
                    (name, (!value.is_empty()).then_some(value))
                })
        };
        if let Some((_, check)) = value_options.iter().find(|(option, _)| *option == name) {
            if !check(attached.or_else(|| args.next())?) {
                return None;
            }
            options.push(name);
        } else if flags.contains(&arg) {
            options.push(arg);
        } else if !arg[1..].chars().all(|flag| short_flags.contains(flag)) {
            return None;
        }
    }
    operands
        .iter()
        .all(|word| !word.is_empty())
        .then_some(Arguments { operands, options })
}

const WC_FLAGS: &[&str] = &["--bytes", "--chars", "--lines", "--words"];

fn source(words: &[&str]) -> bool {
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    let Some(program) = program_name(program) else {
        return false;
    };
    match program {
        "cat" => arguments(args, "AbEnsTtuv", &[], &[]).is_some(),
        "head" | "tail" => line_source(args),
        "nl" => arguments(
            args,
            "",
            &[],
            &[("-b", |style| matches!(style, "a" | "t" | "n"))],
        )
        .is_some(),
        "wc" => arguments(args, "clmwL", WC_FLAGS, &[]).is_some(),
        "sed" => is_sed_print(args),
        "rg" => rg(args),
        "grep" => search(args, "nFiIlcwrRvsHoh", &["--recursive", "--line-number"]),
        "ls" => arguments(args, "aAlhRrdt1FS", &["--all", "--almost-all"], &[]).is_some(),
        "fd" => arguments(
            args,
            "HiIsp",
            &["--hidden", "--no-ignore"],
            &[
                ("-e", literal),
                ("--extension", literal),
                ("-d", is_number),
                ("--max-depth", is_number),
            ],
        )
        .is_some(),
        "find" => find(args),
        "git" => git(args),
        _ => false,
    }
}

fn line_source(args: &[&str]) -> bool {
    let args = match args {
        [count, rest @ ..] if count.strip_prefix('-').is_some_and(is_number) => rest,
        _ => args,
    };
    arguments(
        args,
        "qv",
        &[],
        &[
            ("-n", is_number),
            ("--lines", is_number),
            ("-c", is_number),
            ("--bytes", is_number),
        ],
    )
    .is_some_and(|args| args.operands.iter().all(|word| !word.starts_with('+')))
}

fn rg(args: &[&str]) -> bool {
    let nonempty = |value: &str| !value.is_empty();
    arguments(
        args,
        "iSsFwxnNlcovH",
        &[
            "--files",
            "--no-filename",
            "--hidden",
            "--no-ignore",
            "-u",
            "-uu",
            "--no-heading",
            "--heading",
            "--json",
            "--column",
            "--trim",
        ],
        &[
            ("-g", nonempty),
            ("--glob", nonempty),
            ("--iglob", nonempty),
            ("-t", nonempty),
            ("--type", nonempty),
            ("-T", nonempty),
            ("--type-not", nonempty),
            ("-A", is_number),
            ("-B", is_number),
            ("-C", is_number),
            ("-m", is_number),
            ("--max-count", is_number),
            ("--max-depth", is_number),
            ("-e", |_| true),
            ("--color", |value| value == "never"),
            ("--sort", |value| value == "path"),
        ],
    )
    .is_some_and(|args| {
        let has_pattern_option = args.options.contains(&"-e");
        if args.options.contains(&"--files") {
            !has_pattern_option
        } else {
            has_pattern_option || !args.operands.is_empty()
        }
    })
}

fn search(args: &[&str], short: &str, flags: &[&str]) -> bool {
    arguments(
        args,
        short,
        flags,
        &[
            ("-e", |pattern| !pattern.starts_with("--")),
            ("--regexp", |pattern| !pattern.starts_with("--")),
            ("-m", is_number),
            ("--max-count", is_number),
        ],
    )
    .is_some_and(|args| {
        !args.operands.is_empty()
            || args.options.contains(&"-e")
            || args.options.contains(&"--regexp")
    })
}

fn find(args: &[&str]) -> bool {
    let mut args = args.iter().copied().peekable();
    while args.next_if(|word| *word != "!" && literal(word)).is_some() {}
    while let Some(flag) = args.next() {
        let valid = match flag {
            "-name" | "-iname" | "-path" => args.next().is_some_and(literal),
            "-type" => args
                .next()
                .is_some_and(|kind| matches!(kind, "b" | "c" | "d" | "f" | "l" | "p" | "s")),
            "-mindepth" | "-maxdepth" => args.next().is_some_and(is_number),
            "-print" => true,
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    true
}

fn git(args: &[&str]) -> bool {
    let Some((command, args)) = args.split_first() else {
        return false;
    };
    if args.iter().any(|arg| arg.starts_with("--output")) {
        return false;
    }
    match *command {
        "grep" => search(args, "nFiIlcwvHo", &["--line-number", "--cached"]),
        "ls-files" => arguments(args, "", &[], &[]).is_some(),
        "show" => matches!(args, [object] if object.split_once(':')
            .is_some_and(|(revision, path)| literal(revision) && !path.is_empty())),
        "diff" => {
            arguments(args, "", &["--no-ext-diff", "--no-textconv"], &[]).is_some_and(|args| {
                args.options.contains(&"--no-ext-diff") && args.options.contains(&"--no-textconv")
            })
        }
        "log" => arguments(
            args,
            "0123456789",
            &["--oneline", "--no-merges", "--first-parent"],
            &[("-n", is_number), ("--max-count", is_number)],
        )
        .is_some(),
        "blame" => arguments(args, "", &["--no-textconv"], &[]).is_some_and(|args| {
            args.options.contains(&"--no-textconv") && args.operands.len() == 1
        }),
        _ => false,
    }
}

fn filter(words: &[&str]) -> bool {
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    let Some(program) = program_name(program) else {
        return false;
    };
    match program {
        "head" | "tail" => match args {
            [] => true,
            [count] => ["-n", "--lines=", "-"]
                .into_iter()
                .any(|flag| count.strip_prefix(flag).is_some_and(is_number)),
            ["-n" | "--lines", count] => is_number(count),
            _ => false,
        },
        "wc" => stdin_only(args, "clmwL", WC_FLAGS),
        "sort" => stdin_only(
            args,
            "bdfghiMnruV",
            &["--numeric-sort", "--reverse", "--unique"],
        ),
        "uniq" => matches!(args, [] | ["-c"]),
        "cut" => cut(args),
        "nl" => matches!(args, [] | ["-ba"]),
        "tr" => match args {
            ["-d", set] => literal(set),
            [from, to] => literal(from) && literal(to),
            _ => false,
        },
        _ => false,
    }
}

fn stdin_only(args: &[&str], short: &str, flags: &[&str]) -> bool {
    arguments(args, short, flags, &[]).is_some_and(|args| args.operands.is_empty())
}

fn cut(args: &[&str]) -> bool {
    arguments(
        args,
        "",
        &[],
        &[
            ("-f", selection),
            ("--fields", selection),
            ("-c", selection),
            ("--characters", selection),
            ("-d", |delimiter| delimiter.len() == 1),
            ("--delimiter", |delimiter| delimiter.len() == 1),
        ],
    )
    .is_some_and(|args| {
        args.operands.is_empty()
            && matches!(
                args.options.as_slice(),
                ["-f" | "--fields" | "-c" | "--characters"]
                    | ["-f" | "--fields", "-d" | "--delimiter"]
                    | ["-d" | "--delimiter", "-f" | "--fields"]
            )
    })
}

fn selection(value: &str) -> bool {
    value.split(',').all(|range| match range.split_once('-') {
        Some(("", last)) => is_number(last),
        Some((first, "")) => is_number(first),
        Some((first, last)) => is_number(first) && is_number(last),
        None => is_number(range),
    })
}
