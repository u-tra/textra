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
