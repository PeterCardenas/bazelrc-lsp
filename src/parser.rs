use chumsky::prelude::*;
use chumsky::Parser;

pub type Span = std::ops::Range<usize>;
pub type Spanned<T> = (T, Span);

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Line {
    pub command: Option<Spanned<String>>,
    pub config: Option<Spanned<String>>,
    pub flags: Vec<Spanned<String>>,
    pub comment: Option<Spanned<String>>,
    // The span of this line (without the comment)
    pub span: Span,
}

// Tokenizer and parser for bazelrc files.
//
// The syntax supported by bazelrc is primarily implementation-defined
// and it seems to be a bit ad-hoc.
//
// As such, rather exotic lines like
// > b"uil"d':o'pt --"x"='y'
// are valid. In this case, the line is equivalent to
// > build:opt --x=y
//
// For proper LSP support, we need to track the offsets inside
// the orignal string though. Given that the interesting structural
// parts (e.g., the `build` and `opt`) would be munged together
// in a single token with a single source location range, I decided
// against the classical split betwen tokenizer and parser.
//
// Instead, the tokenizer and parser are combined. We detect most
// valid syntax, but for some more exotic syntax like
// > b"uil"d':o'pt --"x"='y'
// above, we will not be able to extract the correct location
// information.
//
// See rc_file.cc and util/strings.cc from the Bazel source code
pub fn parser() -> impl Parser<char, Vec<Line>, Error = Simple<char>> {
    // The token separators
    let specialchars = " \t\r\n\"\'#";

    // All characters except for separators and `\` characters are part of tokens
    let raw_token_char = filter(|c| *c != '\\' && !specialchars.contains(*c));

    // Characters can be escaped with a `\` (except for newlines; those are treated in escaped_newline)
    let escaped_char = just('\\').ignore_then(filter(|c| *c != '\n' && *c != '\r'));

    // A newline. Either a Windows or a Unix newline
    let newline = just('\n').or(just('\r').ignore_then(just('\n')));

    // Newlines can be escaped using a `\`, but in contrast to other escaped parameters they
    // don't contribute any characters to the token value.
    let escaped_newline = just('\\').ignore_then(newline);

    // A token character can be either a raw character, an escaped character
    // or an escaped newline.
    let token_char = (raw_token_char.or(escaped_char))
        .map(Option::Some)
        .or(escaped_newline.to(Option::<char>::None));

    // A token consists of multiple token_chars
    let unquoted_token_raw = token_char.repeated().at_least(1);

    // Quoted tokens with `"`
    let dquoted_token_raw = just('"')
        .ignore_then(token_char.or(one_of(" \t\'#").map(Option::Some)).repeated())
        .then_ignore(just('"'));

    // Quoted tokens with `'`
    let squoted_token_raw = just('\'')
        .ignore_then(token_char.or(one_of(" \t\"#").map(Option::Some)).repeated())
        .then_ignore(just('\''));

    // Quoted tokens. Either with `"` or with `'`
    let quoted_token_raw = dquoted_token_raw.or(squoted_token_raw);

    // Mixed tokens, consisting of both quoted and unquoted parts
    let mixed_token = unquoted_token_raw
        .or(quoted_token_raw)
        .repeated()
        .at_least(1)
        .flatten()
        .map(|v| v.iter().filter_map(|c| *c).collect::<String>());

    // Tokens are separated by whitespace
    let separator = one_of(" \t").repeated().at_least(1).ignored();

    // Comments go until the end of line.
    // However a newline might be escaped using `\`
    let comment = just('#')
        .ignore_then(escaped_newline.or(one_of("\n\r").not()).repeated())
        .collect::<String>()
        .map_with_span(|v, span| (v, span));

    // A list of flags
    let flags_list = mixed_token
        .clone()
        .map_with_span(|v, span| (v, span))
        .separated_by(separator.clone())
        .allow_leading()
        .allow_trailing();

    // The command name which might be at the beginning of a query
    let command_name = filter(|c: &char| c.is_alphabetic())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .map_with_span(|v, span| (v, span));

    // The command specifier consists of `command` or `command:config` followed by a whitespace
    let command_specifier = separator
        .clone()
        .or_not()
        .ignore_then(command_name)
        .then(
            just(':')
                .ignore_then(mixed_token)
                .map_with_span(|v, span| (v, span))
                .or_not(),
        )
        .then_ignore(separator.or(newline.rewind().ignored()).or(end()));

    // Detect `command` and `command:config` in the beginnig of a line
    let line_content = command_specifier
        .or_not()
        .then(flags_list)
        .map_with_span(|v, s| (v, s))
        .then(comment.or_not())
        .map(|(((command_config, tokens), span), comment)| {
            let (command, config) = command_config.unzip();
            Line {
                command,
                config: config.flatten(),
                flags: tokens,
                comment,
                span,
            }
        });

    line_content
        .separated_by(newline)
        .collect::<Vec<_>>()
        .then_ignore(end())
}

#[test]
fn test_newlines() {
    // Our parser accepts empty strings
    assert_eq!(parser().parse(""), Ok(Vec::from([Line::default()])));

    // `\n` and `\r\n``separate lines.
    // Lines can have leading and trailing whitespace.
    // We also preserve empty lines
    assert_eq!(
        parser().parse("cmd\n\r\n\ncmd -x \n"),
        Ok(Vec::from([
            Line {
                command: Some(("cmd".to_string(), 0..3)),
                ..Default::default()
            },
            // The empty lines are recorded
            Line::default(),
            Line::default(),
            // This line has content again
            Line {
                command: Some(("cmd".to_string(), 7..10)),
                flags: Vec::from([("-x".to_string(), 11..13)]),
                ..Default::default()
            },
            // The final newline is also preserved
            Line::default(),
        ]))
    );
}

#[test]
fn test_command_specifier() {
    // The first token is the command name
    assert_eq!(
        parser().parse("cmd"),
        Ok(Vec::from([Line {
            command: Some(("cmd".to_string(), 0..3)),
            ..Default::default()
        },]))
    );

    // The command name might be followed by `:config-name`
    assert_eq!(
        parser().parse("cmd:my-config"),
        Ok(Vec::from([Line {
            command: Some(("cmd".to_string(), 0..3)),
            config: Some(("my-config".to_string(), 3..13)),
            ..Default::default()
        },]))
    );

    // The config might contain arbitrarily complex escaped tokens
    assert_eq!(
        parser().parse("cmd:my-\\ con'f ig'"),
        Ok(Vec::from([Line {
            command: Some(("cmd".to_string(), 0..3)),
            config: Some(("my- conf ig".to_string(), 3..18)),
            ..Default::default()
        },]))
    );

    // The command combined with some actual arguments
    assert_eq!(
        parser().parse("build:opt --x=y"),
        Ok(Vec::from([Line {
            command: Some(("build".to_string(), 0..5)),
            config: Some(("opt".to_string(), 5..9)),
            flags: Vec::from([("--x=y".to_string(), 10..15)]),
            ..Default::default()
        },]))
    );
}

#[test]
fn test_flag_parsing() {
    let flags_only = |e: &str| {
        parser().parse("command ".to_string() + e).map(|v| {
            v.iter()
                // Remove empty lines
                .filter(|l| !l.flags.is_empty() || l.comment.is_some())
                // Remove positions
                .map(|v2| v2.flags.iter().map(|e| e.0.clone()).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        })
    };
    let single_line = |t: &[String]| Ok(Vec::from([Vec::from(t)]));
    let single_token = |t: String| single_line(&[t]);

    macro_rules! assert_single_flag {
        ($a1:expr, $a2:expr) => {
            assert_eq!(flags_only($a1), single_token($a2.to_string()));
        };
    }

    // A simple token without escaped characters
    assert_single_flag!("abc", "abc");
    // Characters inside tokens can be escaped using `\`
    assert_single_flag!("a\\bc\\d", "abcd");
    // A `\` is escaped using another `\`
    assert_single_flag!("a\\\\b", "a\\b");
    // A `\` can also be used to escape whitespaces or tabs
    assert_single_flag!("a\\ b\\\tc", "a b\tc");

    // A token can contain be escaped using `"`
    assert_single_flag!("\"a b\tc\"", "a b\tc");
    // Instead of `"`, one can also use `'` to escape
    assert_single_flag!("'a b\tc'", "a b\tc");
    // Inside `"`, other `"` can be escaped. `'` can be included unescaped
    assert_single_flag!("\"a\\\"b'c\"", "a\"b'c");
    // Inside `'`, other `'` can be escaped. `"` can be included unescaped
    assert_single_flag!("'a\"b\\'c'", "a\"b'c");

    // Quoted parts can also appear in the middle of tokens
    assert_single_flag!("abc' cd\t e\\''fg\"h i\"j", "abc cd\t e'fgh ij");

    // A whitespace seperates two tokens
    assert_eq!(
        flags_only("ab c"),
        single_line(&["ab".to_string(), "c".to_string()])
    );
    // Instead of a whitespace, one can also use a tab
    assert_eq!(
        flags_only("ab\tc"),
        single_line(&["ab".to_string(), "c".to_string()])
    );
    // Two tokens can also be separated by multiple whitespaces
    assert_eq!(
        flags_only("ab\t \t  c"),
        single_line(&["ab".to_string(), "c".to_string()])
    );
    // Multiple quoted tokens
    assert_eq!(
        flags_only("\"t 1\" 't 2'"),
        single_line(&["t 1".to_string(), "t 2".to_string()])
    );

    // A token can be continued on the next line using a `\`
    assert_single_flag!("a\\\nbc", "abc".to_string());
    // A quoted token does not continue across lines
    assert!(parser().parse("'my\ntoken'").is_err());
    // But a quoted token can contain escaped newlines
    assert_single_flag!("'my\\\ntoken'", "mytoken".to_string());

    // `#` inside a quoted token does not start a token
    assert_single_flag!("'a#c'", "a#c".to_string());
    // `#` can be escaped as part of a token
    assert_single_flag!("a\\#c", "a#c".to_string());
}

#[test]
fn test_comments() {
    // Comments
    assert_eq!(
        parser().parse(" # my comment\n#2nd comment"),
        Ok(Vec::from([
            Line {
                comment: Some((" my comment".to_string(), 1..13)),
                ..Default::default()
            },
            Line {
                comment: Some(("2nd comment".to_string(), 14..26)),
                ..Default::default()
            }
        ]))
    );
    // Comments can be continued across lines with `\`
    assert_eq!(
        parser().parse(" # my\\\nco\\mment"),
        Ok(Vec::from([Line {
            comment: Some((" my\nco\\mment".to_string(), 1..15)),
            ..Default::default()
        }]))
    );

    // Comments can even start in the middle of a token, without a whitespace
    assert_eq!(
        parser().parse("cmd flag#comment"),
        Ok(Vec::from([Line {
            command: Some(("cmd".to_string(), 0..3)),
            flags: Vec::from([("flag".to_string(), 4..8)]),
            comment: Some(("comment".to_string(), 8..16)),
            ..Default::default()
        }]))
    );
}
