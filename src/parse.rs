use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{char, line_ending, multispace0, multispace1, not_line_ending, space0},
    combinator::{all_consuming, map, opt, recognize},
    error::{context, VerboseError, VerboseErrorKind},
    multi::{many0, many1},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};
use std::fmt;

use super::*;

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl From<VerboseError<&str>> for ParseError {
    fn from(err: VerboseError<&str>) -> Self {
        let mut message = String::new();
        let mut line = 0;
        let mut column = 0;

        for (input, error) in err.errors.iter() {
            match error {
                VerboseErrorKind::Context(context) => {
                    message.push_str(&format!("Error: {}\n", context));
                    (line, column) = position_in_input(input, context);
                    break;
                }
                VerboseErrorKind::Char(c) => {
                    message.push_str(&format!("Error: Expected '{}'\n", c));
                    (line, column) = position_in_input(input, &c.to_string());
                    break;
                }
                _ => {
                    message.push_str("Unexpected error\n");
                }
            }
        }

        ParseError {
            message,
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Error at line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl Config {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        match all_consuming(parse_config)(input) {
            Ok((_, config)) => Ok(config),
            Err(nom::Err::Error(e) | nom::Err::Failure(e)) => Err(ParseError::from(e)),
            Err(_) => Err(ParseError {
                message: "Unknown parsing error".to_string(),
                line: 0,
                column: 0,
            }),
        }
    }

    pub fn empty() -> Self {
        Config { matches: vec![] }
    }
}

fn parse_config(input: &str) -> IResult<&str, Config, VerboseError<&str>> {
    //config is a list of matches delimited by optional comments or multispaces
    let (input, matches) = many0(delimited(parse_comment_or_multispace0, parse_match, parse_comment_or_multispace0))(input)?;
    Ok((input, Config { matches }))
}

fn parse_comment<'a>(input: &'a str) -> IResult<&'a str, &'a str, VerboseError<&'a str>> {
    recognize(terminated(
        preceded(tag("//"), not_line_ending),
        opt(line_ending)
    ))(input)
}

fn parse_comment_or_multispace0(input: &str) -> IResult<&str, (), VerboseError<&str>> {
    map(
        many0(alt((
            map(parse_comment, |_| ()),
            map(multispace1, |_| ()), // Changed from multispace0 to multispace1
        ))),
        |_| ()
    )(input)
}

fn parse_match(input: &str) -> IResult<&str, Match, VerboseError<&str>> {
    context(
        "match",
        map(
            tuple((
                take_until("=>"),
                tag("=>"),
                multispace0,
                parse_replacement,
                opt(line_ending)
            )),
            |(trigger, _, _, replacement, _)| Match {
                trigger: trigger.trim().to_string(),
                replacement,
            }
        )
    )(input)
}

fn parse_replacement(input: &str) -> IResult<&str, Replacement, VerboseError<&str>> {
    context(
        "replacement",
        alt((
            parse_dynamic_replacement,
            parse_placeholder_replacement,
            parse_multiline_replacement,
            parse_singleline_replacement,
        ))
    )(input)
}

fn parse_multiline_replacement(input: &str) -> IResult<&str, Replacement, VerboseError<&str>> {
    context(
        "multiline replacement",
        map(
            delimited(char('`'), take_until("`"), char('`')),
            |text: &str| Replacement::Static { text: text.to_string() }
        )
    )(input)
}

fn parse_singleline_replacement(input: &str) -> IResult<&str, Replacement, VerboseError<&str>> {
    context(
        "singleline replacement",
        map(
            not_line_ending,
            |text: &str| Replacement::Static { text: text.trim().to_string() }
        )
    )(input)
}

fn parse_dynamic_replacement(input: &str) -> IResult<&str, Replacement, VerboseError<&str>> {
    context(
        "dynamic replacement",
        map(
            delimited(
                tag("[javascript]"),
                delimited(char('{'), take_until("}"), char('}')),
                opt(line_ending)
            ),
            |action: &str| Replacement::Dynamic { action: action.trim().to_string() }
        )
    )(input)
}

fn parse_placeholder_replacement(input: &str) -> IResult<&str, Replacement, VerboseError<&str>> {
    context(
        "placeholder replacement",
        map(
            delimited(char('{'), take_until("}"), char('}')),
            |name: &str| Replacement::Dynamic { action: name.trim().to_string() }
        )
    )(input)
}

fn position_in_input(full_input: &str, error_input: &str) -> (usize, usize) {
    let full_input_lines: Vec<&str> = full_input.lines().collect();
    let error_position = full_input.len() - error_input.len();

    let mut line = 0;
    let mut column = 0;
    let mut cumulative_len = 0;

    for (i, l) in full_input_lines.iter().enumerate() {
        cumulative_len += l.len() + 1; // +1 for the newline character
        if cumulative_len > error_position {
            line = i + 1;
            column = error_position - (cumulative_len - l.len() - 1);
            break;
        }
    }

    (line, column)
}
#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_CONFIG: &str = r#"
// this is textra config file
// you can add your own triggers and replacements here
// when you type the text before `=>` it will be replaced with the text that follows
// it's as simple as that!


btw => by the way
:date => {date.now()}
:time => {time.now()}
:email => example@example.com
:psswd => 0nceUpon@TimeInPluto  
pfa => please find the attached information as requested
pftb => please find the below information as required
:tst => `twinkle twinkle little star, how i wonder what you are,
up above the world so high,
like a diamond in the sky`
ccc => continue writing complete code without skipping anything
//we can also write complex code that we want to execute

:ping => [javascript]{
    let pr = await network.ping("www.google.com");
    return "I pinged Google and it responded $pr";
}
"#;

    #[test]
    fn test_parse_single_line_comment() {
        let input = "// this is a comment\nnext line";
        let result = parse_comment(input);
        assert_eq!(result, Ok(("next line", "// this is a comment\n")));
    }

    #[test]
    fn test_parse_multiple_comments() {
        let input = "// comment 1\n// comment 2\nactual content";
        let result = parse_comment_or_multispace0(input);
        assert_eq!(result, Ok(("actual content", ())));
    }

    #[test]
    fn test_parse_simple_static_replacement() {
        let input = "by the way";
        let result = parse_singleline_replacement(input);
        assert_eq!(result, Ok(("", Replacement::Static { text: "by the way".to_string() })));
    }

    #[test]
    fn test_parse_multiline_static_replacement() {
        let input = "`line 1\nline 2\nline 3`";
        let result = parse_multiline_replacement(input);
        assert_eq!(result, Ok(("", Replacement::Static { text: "line 1\nline 2\nline 3".to_string() })));
    }

    #[test]
    fn test_parse_placeholder_replacement() {
        let input = "{date.now()}";
        let result = parse_placeholder_replacement(input);
        assert_eq!(result, Ok(("", Replacement::Dynamic { action: "date.now()".to_string() })));
    }

    #[test]
    fn test_parse_dynamic_javascript_replacement() {
        let input = r#"[javascript]{
    let pr = await network.ping("www.google.com");
    return "I pinged Google and it responded $pr";
}"#;
        let result = parse_dynamic_replacement(input);
        assert!(result.is_ok());
        let (remainder, replacement) = result.unwrap();
        assert_eq!(remainder, "");
        match replacement {
            Replacement::Dynamic { action } => {
                assert!(action.contains("let pr = await network.ping("));
                assert!(action.contains("return \"I pinged Google and it responded $pr\";"));
            },
            _ => panic!("Expected Dynamic replacement"),
        }
    }

    #[test]
    fn test_parse_single_match() {
        let input = "btw => by the way\n";
        let result = parse_match(input);
        assert_eq!(
            result,
            Ok((
                "",
                Match {
                    trigger: "btw".to_string(),
                    replacement: Replacement::Static { text: "by the way".to_string() }
                }
            ))
        );
    }

    #[test]
    fn test_parse_match_with_placeholder() {
        let input = ":date => {date.now()}\n";
        let result = parse_match(input);
        assert_eq!(
            result,
            Ok((
                "",
                Match {
                    trigger: ":date".to_string(),
                    replacement: Replacement::Dynamic { action: "date.now()".to_string() }
                }
            ))
        );
    }

    #[test]
    fn test_parse_match_with_javascript() {
        let input = r#":ping => [javascript]{
    let pr = await network.ping("www.google.com");
    return "I pinged Google and it responded $pr";
}
"#;
        let result = parse_match(input);
        assert!(result.is_ok());
        let (remainder, match_result) = result.unwrap();
        assert_eq!(remainder, "");
        assert_eq!(match_result.trigger, ":ping");
        match match_result.replacement {
            Replacement::Dynamic { action } => {
                assert!(action.contains("let pr = await network.ping("));
                assert!(action.contains("return \"I pinged Google and it responded $pr\";"));
            },
            _ => panic!("Expected Dynamic replacement"),
        }
    }

    #[test]
    fn test_parse_full_config() {
        let result = Config::parse(DEFAULT_CONFIG);
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.matches.len(), 10);
        
        // Test some specific matches
        assert_eq!(config.matches[0].trigger, "btw");
        assert_eq!(config.matches[0].replacement, Replacement::Static { text: "by the way".to_string() });
        
        assert_eq!(config.matches[1].trigger, ":date");
        assert_eq!(config.matches[1].replacement, Replacement::Dynamic { action: "date.now()".to_string() });
        
        assert_eq!(config.matches[7].trigger, ":tst");
        match &config.matches[7].replacement {
            Replacement::Static { text } => {
                assert!(text.contains("twinkle twinkle little star"));
                assert!(text.contains("like a diamond in the sky"));
            },
            _ => panic!("Expected Static replacement for :tst"),
        }
        
        assert_eq!(config.matches[9].trigger, ":ping");
        match &config.matches[9].replacement {
            Replacement::Dynamic { action } => {
                assert!(action.contains("let pr = await network.ping("));
                assert!(action.contains("return \"I pinged Google and it responded $pr\";"));
            },
            _ => panic!("Expected Dynamic replacement for :ping"),
        }
    }
}