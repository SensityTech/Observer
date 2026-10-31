//! A resource's manifest read as text: the few directives a change is told by. It is never run, so
//! a value the manifest builds in Lua is only known to be computed.

use observer_domain::changes::Manifest;

/// The longest value kept, in characters.
const MAX_VALUE: usize = 256;
const MAX_GAMES: usize = 4;
const DIRECTIVES: [&str; 7] = [
    "version",
    "fx_version",
    "game",
    "games",
    "author",
    "description",
    "lua54",
];

/// Reads the directives of `text`, the manifest `file` of a resource. The first occurrence of each
/// directive counts, as the first value of a list does.
pub fn parse(file: &str, text: &str) -> Manifest {
    let mut manifest = Manifest {
        file: file.to_owned(),
        ..Manifest::default()
    };
    let mut seen: Vec<&str> = Vec::new();
    let mut lexer = Lexer { text, at: 0 };
    let mut previous: Option<Token> = None;
    while let Some(token) = lexer.next() {
        let directive = match &token {
            Token::Name(name) => DIRECTIVES.iter().copied().find(|known| known == name),
            _ => None,
        };
        // `x.version 'a'` and `x:version 'a'` are calls on something else.
        let after_access = matches!(previous, Some(Token::Symbol('.' | ':')));
        previous = Some(token);
        let Some(directive) = directive.filter(|_| !after_access) else {
            continue;
        };
        let value = lexer.value();
        if seen.contains(&directive) {
            continue;
        }
        seen.push(directive);
        let Some(values) = value else {
            manifest.computed.push(directive.to_owned());
            continue;
        };
        let first = values.first().map(|value| cut(value));
        match directive {
            "version" => manifest.version = first,
            "fx_version" => manifest.fx_version = first,
            "author" => manifest.author = first,
            "description" => manifest.description = first,
            "lua54" => manifest.lua54 = first,
            _ => {
                for game in values.iter().take(MAX_GAMES) {
                    let game = cut(game);
                    if !manifest.games.contains(&game) {
                        manifest.games.push(game);
                    }
                }
            }
        }
    }
    manifest
}

fn cut(value: &str) -> String {
    value.chars().take(MAX_VALUE).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name(String),
    Text(String),
    Symbol(char),
}

struct Lexer<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Lexer<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn next(&mut self) -> Option<Token> {
        self.skip_blanks();
        let rest = self.rest();
        let first = rest.chars().next()?;
        if first == '_' || first.is_ascii_alphabetic() {
            let end = rest
                .find(|c: char| c != '_' && !c.is_ascii_alphanumeric())
                .unwrap_or(rest.len());
            self.at += end;
            return Some(Token::Name(rest[..end].to_owned()));
        }
        if let Some(text) = self.string() {
            return Some(Token::Text(text));
        }
        self.at += first.len_utf8();
        Some(Token::Symbol(first))
    }

    /// The value after a directive: a string, a string in parentheses, or the strings of a table.
    /// `None` when it is anything else, which the manifest computes.
    fn value(&mut self) -> Option<Vec<String>> {
        let start = self.at;
        let value = self.literal();
        if value.is_none() {
            self.at = start;
        }
        value
    }

    fn literal(&mut self) -> Option<Vec<String>> {
        match self.next()? {
            Token::Text(text) => self.alone(vec![text]),
            Token::Symbol('(') => {
                let inner = match self.next()? {
                    Token::Text(text) => vec![text],
                    Token::Symbol('{') => self.table()?,
                    _ => return None,
                };
                (self.next()? == Token::Symbol(')')).then_some(())?;
                self.alone(inner)
            }
            Token::Symbol('{') => {
                let values = self.table()?;
                self.alone(values)
            }
            _ => None,
        }
    }

    /// The strings of a table up to its `}`; `None` when it holds anything else.
    fn table(&mut self) -> Option<Vec<String>> {
        let mut values = Vec::new();
        loop {
            match self.next()? {
                Token::Text(text) => values.push(text),
                Token::Symbol(',' | ';') => {}
                Token::Symbol('}') => return Some(values),
                _ => return None,
            }
        }
    }

    /// The value, unless what follows joins it to more: `'1.' .. build` is computed.
    fn alone(&mut self, values: Vec<String>) -> Option<Vec<String>> {
        self.skip_blanks();
        let rest = self.rest();
        let joined = rest.starts_with("..")
            || rest.starts_with('+')
            || rest.starts_with('[')
            || rest.starts_with(':')
            || (rest.starts_with('.') && !rest.starts_with(".."));
        (!joined).then_some(values)
    }

    fn skip_blanks(&mut self) {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.at += rest.len() - trimmed.len();
            if let Some(comment) = trimmed.strip_prefix("--") {
                self.at += 2;
                if let Some(level) = long_bracket(comment) {
                    self.at += level + 2;
                    let close = format!("]{}]", "=".repeat(level));
                    let body = self.rest();
                    self.at += body.find(&close).map_or(body.len(), |end| end + close.len());
                } else {
                    self.at += comment.find('\n').unwrap_or(comment.len());
                }
                continue;
            }
            return;
        }
    }

    /// A string literal at the cursor: `'...'`, `"..."` or a long bracket `[[...]]`.
    fn string(&mut self) -> Option<String> {
        let rest = self.rest();
        if let Some(level) = long_bracket(rest) {
            let close = format!("]{}]", "=".repeat(level));
            let body = &rest[level + 2..];
            let end = body.find(&close)?;
            // A newline right after the opening bracket is not part of the string.
            let text = body[..end]
                .strip_prefix("\r\n")
                .or_else(|| body[..end].strip_prefix('\n'))
                .unwrap_or(&body[..end]);
            let text = text.to_owned();
            self.at += level + 2 + end + close.len();
            return Some(text);
        }
        let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
        let mut text = String::new();
        let mut chars = rest.char_indices().skip(1);
        while let Some((index, c)) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some((_, 'n')) => text.push('\n'),
                    Some((_, 't')) => text.push('\t'),
                    Some((_, escaped)) => text.push(escaped),
                    None => return None,
                },
                '\n' => return None,
                c if c == quote => {
                    self.at += index + 1;
                    return Some(text);
                }
                c => text.push(c),
            }
        }
        None
    }
}

/// The level of a long bracket opening `text`: 0 for `[[`, 1 for `[=[`.
fn long_bracket(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('[')?;
    let level = rest.len() - rest.trim_start_matches('=').len();
    rest[level..].starts_with('[').then_some(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_directive_is_read_in_its_usual_forms() {
        let manifest = parse(
            "fxmanifest.lua",
            "fx_version 'cerulean'\n\
             games { 'gta5', \"rdr3\" }\n\
             author \"Sensity\"\n\
             description [[A long\ndescription]]\n\
             version('1.3.0')\n\
             lua54 'yes'\n\
             client_script 'version.lua'\n",
        );

        assert_eq!(manifest.fx_version.as_deref(), Some("cerulean"));
        assert_eq!(manifest.games, ["gta5", "rdr3"]);
        assert_eq!(manifest.author.as_deref(), Some("Sensity"));
        assert_eq!(manifest.description.as_deref(), Some("A long\ndescription"));
        assert_eq!(manifest.version.as_deref(), Some("1.3.0"));
        assert_eq!(manifest.lua54.as_deref(), Some("yes"));
        assert_eq!(manifest.computed, Vec::<String>::new());
    }

    #[test]
    fn a_single_game_and_the_first_of_each_directive_count() {
        let manifest = parse(
            "__resource.lua",
            "game 'rdr3'\nversion '1.0.0'\nversion '2.0.0'\n",
        );

        assert_eq!(manifest.file, "__resource.lua");
        assert_eq!(manifest.games, ["rdr3"]);
        assert_eq!(manifest.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn a_value_built_in_lua_is_computed() {
        for text in [
            "version GetConvar('v', '1')",
            "version '1.' .. build",
            "local v = '1'\nversion v",
            "version ('1'):upper()",
        ] {
            let manifest = parse("fxmanifest.lua", text);
            assert_eq!(manifest.version, None, "{text}");
            assert_eq!(manifest.computed, ["version"], "{text}");
        }
    }

    #[test]
    fn comments_and_method_calls_are_not_directives() {
        let manifest = parse(
            "fxmanifest.lua",
            "-- version '9.9.9'\n--[[ version '8.8.8' ]]\nconfig.version '7'\nversion '1.2'\n",
        );

        assert_eq!(manifest.version.as_deref(), Some("1.2"));
    }

    #[test]
    fn long_strings_of_any_level_are_read() {
        let manifest = parse("fxmanifest.lua", "description [==[a ]] b]==]\nversion [[\n1.0]]");

        assert_eq!(manifest.description.as_deref(), Some("a ]] b"));
        assert_eq!(manifest.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn values_are_cut_at_256_characters() {
        let long = "x".repeat(300);
        let manifest = parse("fxmanifest.lua", &format!("author '{long}'"));

        assert_eq!(manifest.author.map(|author| author.chars().count()), Some(256));
    }

    #[test]
    fn an_unterminated_string_reads_as_computed() {
        let manifest = parse("fxmanifest.lua", "version '1.0\nauthor 'a'");

        assert_eq!(manifest.version, None);
        assert_eq!(manifest.author.as_deref(), Some("a"));
    }
}
