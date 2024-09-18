use pest::Parser;
use pest_derive::Parser;
use std::collections::HashMap;
use pest::error::Error;
use pest::iterators::Pair;

#[derive(Parser)]
#[grammar = "textra.pest"]
struct TextraParser;

#[derive(Debug, Clone)]
pub struct TextraConfig {
    pub metadata: HashMap<String, String>,
    pub documentation: Vec<String>,
    pub rules: Vec<TextraRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextraRule {
    pub triggers: Vec<String>,
    pub replacement: Replacement,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Replacement {
    Simple(String),
    Multiline(String),
    Code { language: String, content: String },
}

pub type ParseError = pest::error::Error<Rule>;

pub fn parse_textra_config(input: &str) -> Result<TextraConfig, Error<Rule>> {
    let mut config = TextraConfig {
        metadata: HashMap::new(),
        documentation: Vec::new(),
        rules: Vec::new(),
    };

    let pairs = TextraParser::parse(Rule::file, input)?;

    for pair in pairs {
        match pair.as_rule() {
            Rule::file => {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::metadata => parse_metadata(&mut config, inner_pair),
                        Rule::documentation => parse_documentation(&mut config, inner_pair),
                        Rule::rule => parse_rule(&mut config, inner_pair),
                        Rule::EOI => {}
                        _ => unreachable!(),
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(config)
}

fn parse_metadata(config: &mut TextraConfig, pair: Pair<Rule>) {
    let mut inner = pair.into_inner();
    let key = inner.next().unwrap().as_str().to_string();
    let value = inner.next().unwrap().as_str().to_string();
    config.metadata.insert(key, value);
}

fn parse_documentation(config: &mut TextraConfig, pair: Pair<Rule>) {
    let doc = pair.into_inner().next().unwrap().as_str().trim().to_string();
    config.documentation.push(doc);
}

fn parse_rule(config: &mut TextraConfig, pair: Pair<Rule>) {
    let mut inner = pair.into_inner();
    let triggers = parse_triggers(inner.next().unwrap());
    let replacement = parse_replacement(inner.next().unwrap());

    config.rules.push(TextraRule {
        triggers,
        replacement,
    });
}

fn parse_triggers(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .map(|trigger| trigger.as_str().trim().to_string())
        .collect()
}

fn parse_replacement(pair: Pair<Rule>) -> Replacement {
    match pair.as_rule() {
        Rule::replacement => {
            let inner = pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::simple_replacement => Replacement::Simple(inner.as_str().to_string()),
                Rule::multiline_replacement => {
                    let content = inner.into_inner().next().unwrap().as_str().to_string();
                    Replacement::Multiline(content)
                }
                Rule::code_replacement => {
                    let mut code_inner = inner.into_inner();
                    let language = code_inner.next().unwrap().as_str().trim().to_string();
                    let content = code_inner.next().unwrap().as_str().to_string();
                    Replacement::Code { language, content }
                }
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
}

pub fn serialize_textra_config(config: &TextraConfig) -> String {
    let mut output = String::new();

    for (key, value) in &config.metadata {
        output.push_str(&format!("///{key}:{value}\n"));
    }

    for doc in &config.documentation {
        output.push_str(&format!("/// {doc}\n"));
    }

    for rule in &config.rules {
        let triggers = rule.triggers.join(" | ");
        let replacement = match &rule.replacement {
            Replacement::Simple(s) => s.to_string(),
            Replacement::Multiline(s) => format!("`{s}`"),
            Replacement::Code { language, content } => format!("```{language}\n{content}```"),
        };
        output.push_str(&format!("{triggers} => {replacement}\n"));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metadata() {
        let input = "///name:Textra Config Example\n";
        let config = parse_textra_config(input).expect("Failed to parse metadata");

        assert_eq!(config.metadata.get("name"), Some(&"Textra Config Example".to_string()));
    }

    #[test]
    fn test_parse_documentation() {
        let input = "/// This is a Textra configuration file.\n";
        let config = parse_textra_config(input).expect("Failed to parse documentation");

        assert_eq!(config.documentation, vec!["This is a Textra configuration file.".to_string()]);
    }

    #[test]
    fn test_parse_simple_rule() {
        let input = "btw => by the way\n";
        let config = parse_textra_config(input).expect("Failed to parse simple rule");

        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].triggers, vec!["btw".to_string()]);
        assert_eq!(config.rules[0].replacement, Replacement::Simple("by the way".to_string()));
    }

    #[test]
    fn test_parse_multiple_triggers() {
        let input = ":email | :mail => a@xo.rs\n";
        let config = parse_textra_config(input).expect("Failed to parse multiple triggers");

        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].triggers, vec![":email".to_string(), ":mail".to_string()]);
        assert_eq!(config.rules[0].replacement, Replacement::Simple("a@xo.rs".to_string()));
    }

    #[test]
    fn test_parse_multiline_replacement() {
        let input = ":tst => `twinkle twinkle little star,\nhow i wonder what you are`\n";
        let config = parse_textra_config(input).expect("Failed to parse multiline replacement");

        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].triggers, vec![":tst".to_string()]);
        assert_eq!(
            config.rules[0].replacement,
            Replacement::Multiline("twinkle twinkle little star,\nhow i wonder what you are".to_string())
        );
    }

    #[test]
    fn test_parse_code_replacement() {
        let input = ":date => ```javascript\nreturn format.date(date.now(), \"YYYY-MM-DD\");\n```\n";
        let config = parse_textra_config(input).expect("Failed to parse code replacement");

        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].triggers, vec![":date".to_string()]);
        assert_eq!(
            config.rules[0].replacement,
            Replacement::Code {
                language: "javascript".to_string(),
                content: "return format.date(date.now(), \"YYYY-MM-DD\");\n".to_string()
            }
        );
    }
}
