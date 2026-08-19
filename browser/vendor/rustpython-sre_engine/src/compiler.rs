//! CPython 3.12 `re` parser/compiler compatibility layer.
//!
//! The matcher in this crate consumes CPython SRE bytecode. Upstream leaves
//! parsing and compilation to Python's `Lib/re`; this module ports that
//! observable subset directly so the browser worker does not embed Python.

use crate::{MAXGROUPS, MAXREPEAT, SreAtCode, SreCatCode, SreOpcode};
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::fmt;

const FLAG_TEMPLATE: u16 = 1;
const FLAG_IGNORECASE: u16 = 2;
const FLAG_LOCALE: u16 = 4;
const FLAG_MULTILINE: u16 = 8;
const FLAG_DOTALL: u16 = 16;
const FLAG_UNICODE: u16 = 32;
const FLAG_VERBOSE: u16 = 64;
const FLAG_DEBUG: u16 = 128;
const FLAG_ASCII: u16 = 256;
const TYPE_FLAGS: u16 = FLAG_ASCII | FLAG_LOCALE | FLAG_UNICODE;
const GLOBAL_FLAGS: u16 = FLAG_DEBUG | FLAG_TEMPLATE;
const MAX_WIDTH: u64 = u64::MAX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    message: String,
    position: Option<usize>,
}

impl Error {
    fn at(message: impl Into<String>, position: usize) -> Self {
        Self {
            message: message.into(),
            position: Some(position),
        }
    }

    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(position) = self.position {
            write!(f, " at position {position}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Compiled {
    pub codes: Vec<u32>,
    pub groups: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Literal(u32),
    NotLiteral(u32),
    Set(Vec<SetOp>),
    Any,
    Repeat {
        kind: RepeatKind,
        min: usize,
        max: usize,
        body: Pattern,
    },
    Subpattern {
        group: Option<usize>,
        add_flags: u16,
        del_flags: u16,
        body: Pattern,
    },
    Atomic(Pattern),
    Assert {
        negative: bool,
        behind: bool,
        body: Pattern,
    },
    At(u32),
    Branch(Vec<Pattern>),
    GroupRef(usize),
    GroupRefExists {
        group: usize,
        yes: Pattern,
        no: Option<Pattern>,
    },
}

type Pattern = Vec<Op>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepeatKind {
    Greedy,
    Lazy,
    Possessive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SetOp {
    Literal(u32),
    Range(u32, u32),
    Category(u32),
    Negate,
}

#[derive(Default)]
struct ParseState {
    flags: u16,
    group_names: BTreeMap<String, usize>,
    group_widths: Vec<Option<(u64, u64)>>,
    lookbehind_groups: Option<usize>,
    forward_group_refs: BTreeMap<usize, usize>,
}

impl ParseState {
    fn new(flags: u16) -> Self {
        Self {
            flags,
            group_widths: vec![None],
            ..Self::default()
        }
    }

    fn open_group(&mut self, name: Option<String>) -> Result<usize, Error> {
        let group = self.group_widths.len();
        self.group_widths.push(None);
        if self.group_widths.len() > MAXGROUPS {
            return Err(Error::plain("too many groups"));
        }
        if let Some(name) = name {
            if let Some(previous) = self.group_names.get(&name) {
                return Err(Error::plain(format!(
                    "redefinition of group name '{name}' as group {group}; was group {previous}"
                )));
            }
            self.group_names.insert(name, group);
        }
        Ok(group)
    }

    fn group_is_closed(&self, group: usize) -> bool {
        self.group_widths.get(group).is_some_and(Option::is_some)
    }

    fn check_lookbehind_group(&self, group: usize, position: usize) -> Result<(), Error> {
        if let Some(first_lookbehind_group) = self.lookbehind_groups {
            if !self.group_is_closed(group) {
                return Err(Error::at("cannot refer to an open group", position));
            }
            if group >= first_lookbehind_group {
                return Err(Error::at(
                    "cannot refer to group defined in the same lookbehind subpattern",
                    position,
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Tokenizer {
    chars: Vec<char>,
    position: usize,
}

impl Tokenizer {
    fn new(pattern: &str) -> Self {
        Self {
            chars: pattern.chars().collect(),
            position: 0,
        }
    }

    fn token(&self) -> Result<Option<String>, Error> {
        let Some(&ch) = self.chars.get(self.position) else {
            return Ok(None);
        };
        if ch != '\\' {
            return Ok(Some(ch.into()));
        }
        let Some(&escaped) = self.chars.get(self.position + 1) else {
            return Err(Error::at("bad escape (end of pattern)", self.position));
        };
        Ok(Some(format!("\\{escaped}")))
    }

    fn get(&mut self) -> Result<Option<String>, Error> {
        let token = self.token()?;
        if let Some(token) = &token {
            self.position += token.chars().count();
        }
        Ok(token)
    }

    fn matches(&mut self, expected: &str) -> Result<bool, Error> {
        if self.token()?.as_deref() == Some(expected) {
            self.get()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn tell(&self) -> usize {
        self.position
    }

    fn seek(&mut self, position: usize) {
        self.position = position;
    }

    fn get_while(&mut self, limit: usize, allowed: fn(char) -> bool) -> Result<String, Error> {
        let mut out = String::new();
        for _ in 0..limit {
            let Some(token) = self.token()? else { break };
            let mut chars = token.chars();
            let ch = chars.next().expect("nonempty token");
            if chars.next().is_some() || !allowed(ch) {
                break;
            }
            out.push(ch);
            self.get()?;
        }
        Ok(out)
    }

    fn get_until(&mut self, terminator: &str, name: &str) -> Result<String, Error> {
        let mut out = String::new();
        loop {
            let position = self.tell();
            let Some(token) = self.get()? else {
                let message = if out.is_empty() {
                    format!("missing {name}")
                } else {
                    format!("missing {terminator}, unterminated name")
                };
                return Err(Error::at(
                    message,
                    position.saturating_sub(out.chars().count()),
                ));
            };
            if token == terminator {
                if out.is_empty() {
                    return Err(Error::at(format!("missing {name}"), position));
                }
                return Ok(out);
            }
            out.push_str(&token);
        }
    }
}

pub fn compile(pattern: &str, case_insensitive: bool) -> Result<Compiled, Error> {
    let flags = if case_insensitive { FLAG_IGNORECASE } else { 0 };
    let mut parser = Parser {
        source: Tokenizer::new(pattern),
        state: ParseState::new(flags),
    };
    let parsed = parser.parse_sub(false, 0)?;
    parser.state.flags = fix_flags(parser.state.flags)?;
    if parser.source.token()?.is_some() {
        return Err(Error::at("unbalanced parenthesis", parser.source.tell()));
    }
    for (&group, &position) in &parser.state.forward_group_refs {
        if group >= parser.state.group_widths.len() {
            return Err(Error::at(
                format!("invalid group reference {group}"),
                position,
            ));
        }
    }

    let mut codes = Vec::new();
    let (min, max) = width(&parsed, &parser.state);
    emit(&mut codes, SreOpcode::INFO);
    let info_skip = codes.len();
    codes.extend([
        0,
        0,
        min.min(u32::MAX as u64) as u32,
        max.min(u32::MAX as u64) as u32,
    ]);
    if min != 0
        && let Some(set) = first_charset(&parsed)
    {
        codes[info_skip + 1] = crate::SreInfo::CHARSET.bits();
        let mut compiled_set = Vec::new();
        compile_set(set, parser.state.flags, &mut compiled_set);
        codes.extend_from_slice(&compiled_set[2..]);
    }
    codes[info_skip] = (codes.len() - info_skip) as u32;
    compile_pattern(&parsed, parser.state.flags, &parser.state, &mut codes)?;
    emit(&mut codes, SreOpcode::SUCCESS);
    Ok(Compiled {
        codes,
        groups: parser.state.group_widths.len() - 1,
    })
}

struct Parser {
    source: Tokenizer,
    state: ParseState,
}

impl Parser {
    fn error(&self, message: impl Into<String>, offset: usize) -> Error {
        Error::at(message, self.source.tell().saturating_sub(offset))
    }

    fn parse_sub(&mut self, verbose: bool, nested: usize) -> Result<Pattern, Error> {
        let mut branches = Vec::new();
        loop {
            let first = nested == 0 && branches.is_empty();
            branches.push(self.parse_sequence(verbose, nested + 1, first)?);
            if !self.source.matches("|")? {
                break;
            }
        }
        if branches.len() == 1 {
            Ok(branches.pop().expect("one branch"))
        } else {
            Ok(vec![Op::Branch(branches)])
        }
    }

    fn parse_sequence(
        &mut self,
        mut verbose: bool,
        nested: usize,
        first: bool,
    ) -> Result<Pattern, Error> {
        let mut out = Vec::new();
        while let Some(token) = self.source.token()? {
            if token == "|" || token == ")" {
                break;
            }
            self.source.get()?;

            if verbose {
                let ch = one_char(&token);
                if ch.is_some_and(is_verbose_whitespace) {
                    continue;
                }
                if token == "#" {
                    loop {
                        match self.source.get()? {
                            None => break,
                            Some(value) if value == "\n" => break,
                            _ => {}
                        }
                    }
                    continue;
                }
            }

            if token.starts_with('\\') {
                out.push(self.parse_escape(&token, false)?);
                continue;
            }
            if !".\\[{()*+?^$|".contains(&token) {
                out.push(Op::Literal(one_char(&token).expect("literal") as u32));
                continue;
            }
            match token.as_str() {
                "[" => out.push(self.parse_set()?),
                "*" | "+" | "?" | "{" => self.parse_repeat(&mut out, &token)?,
                "." => out.push(Op::Any),
                "(" => {
                    if let Some(op) = self.parse_group(&out, &mut verbose, nested, first)? {
                        out.push(op);
                    }
                }
                "^" => out.push(Op::At(SreAtCode::BEGINNING as u32)),
                "$" => out.push(Op::At(SreAtCode::END as u32)),
                _ => unreachable!("special token {token}"),
            }
        }
        Ok(out)
    }

    fn parse_set(&mut self) -> Result<Op, Error> {
        let start = self.source.tell() - 1;
        let negate = self.source.matches("^")?;
        let mut items = Vec::new();
        loop {
            let Some(token) = self.source.get()? else {
                return Err(Error::at("unterminated character set", start));
            };
            if token == "]" && !items.is_empty() {
                break;
            }
            let first = self.parse_set_atom(&token)?;
            if self.source.matches("-")? {
                let Some(second_token) = self.source.get()? else {
                    return Err(Error::at("unterminated character set", start));
                };
                if second_token == "]" {
                    items.push(first);
                    items.push(SetOp::Literal('-' as u32));
                    break;
                }
                let second = self.parse_set_atom(&second_token)?;
                let (SetOp::Literal(low), SetOp::Literal(high)) = (&first, &second) else {
                    return Err(self.error(
                        format!("bad character range {token}-{second_token}"),
                        token.chars().count() + 1 + second_token.chars().count(),
                    ));
                };
                if high < low {
                    return Err(self.error(
                        format!("bad character range {token}-{second_token}"),
                        token.chars().count() + 1 + second_token.chars().count(),
                    ));
                }
                items.push(SetOp::Range(*low, *high));
            } else {
                items.push(first);
            }
        }
        dedup_set(&mut items);
        if items.len() == 1
            && let SetOp::Literal(value) = items[0]
        {
            return Ok(if negate {
                Op::NotLiteral(value)
            } else {
                Op::Literal(value)
            });
        }
        if negate {
            items.insert(0, SetOp::Negate);
        }
        Ok(Op::Set(items))
    }

    fn parse_set_atom(&mut self, token: &str) -> Result<SetOp, Error> {
        if token.starts_with('\\') {
            match self.parse_escape(token, true)? {
                Op::Literal(value) => Ok(SetOp::Literal(value)),
                Op::Set(mut values) if values.len() == 1 => Ok(values.remove(0)),
                _ => unreachable!("class escape shape"),
            }
        } else {
            Ok(SetOp::Literal(one_char(token).expect("set token") as u32))
        }
    }

    fn parse_repeat(&mut self, out: &mut Pattern, token: &str) -> Result<(), Error> {
        let here = self.source.tell();
        let (mut min, mut max) = match token {
            "?" => (0, 1),
            "*" => (0, MAXREPEAT),
            "+" => (1, MAXREPEAT),
            "{" => {
                if self.source.matches("}")? {
                    out.push(Op::Literal('{' as u32));
                    return Ok(());
                }
                let low = self.source.get_while(usize::MAX, is_digit)?;
                let high = if self.source.matches(",")? {
                    self.source.get_while(usize::MAX, is_digit)?
                } else {
                    low.clone()
                };
                if !self.source.matches("}")? {
                    out.push(Op::Literal('{' as u32));
                    self.source.seek(here);
                    return Ok(());
                }
                let min = parse_repeat_bound(&low)?.unwrap_or(0);
                let max = parse_repeat_bound(&high)?.unwrap_or(MAXREPEAT);
                if max < min {
                    return Err(self.error(
                        "min repeat greater than max repeat",
                        self.source.tell() - here,
                    ));
                }
                (min, max)
            }
            _ => unreachable!(),
        };
        // Keep the names mutable to mirror the parser's bound handling while
        // letting rustc prove both values are initialized.
        min = min.min(MAXREPEAT);
        max = max.min(MAXREPEAT);
        let Some(previous) = out.pop() else {
            return Err(self.error("nothing to repeat", self.source.tell() - here + token.len()));
        };
        if matches!(previous, Op::At(_)) {
            out.push(previous);
            return Err(self.error("nothing to repeat", self.source.tell() - here + token.len()));
        }
        if matches!(previous, Op::Repeat { .. }) {
            out.push(previous);
            return Err(self.error("multiple repeat", self.source.tell() - here + token.len()));
        }
        let kind = if self.source.matches("?")? {
            RepeatKind::Lazy
        } else if self.source.matches("+")? {
            RepeatKind::Possessive
        } else {
            RepeatKind::Greedy
        };
        out.push(Op::Repeat {
            kind,
            min,
            max,
            body: vec![previous],
        });
        Ok(())
    }

    fn parse_group(
        &mut self,
        prefix: &Pattern,
        verbose: &mut bool,
        nested: usize,
        first: bool,
    ) -> Result<Option<Op>, Error> {
        let start = self.source.tell() - 1;
        let mut capture = true;
        let mut atomic = false;
        let mut name = None;
        let mut add_flags = 0;
        let mut del_flags = 0;
        if self.source.matches("?")? {
            let Some(mut extension) = self.source.get()? else {
                return Err(self.error("unexpected end of pattern", 0));
            };
            match extension.as_str() {
                "P" => {
                    if self.source.matches("<")? {
                        let group_name = self.source.get_until(">", "group name")?;
                        self.check_group_name(&group_name, 1)?;
                        name = Some(group_name);
                    } else if self.source.matches("=")? {
                        let group_name = self.source.get_until(")", "group name")?;
                        self.check_group_name(&group_name, 1)?;
                        let Some(&group) = self.state.group_names.get(&group_name) else {
                            return Err(self.error(
                                format!("unknown group name '{group_name}'"),
                                group_name.chars().count() + 1,
                            ));
                        };
                        if !self.state.group_is_closed(group) {
                            return Err(
                                self.error("cannot refer to an open group", group_name.len() + 1)
                            );
                        }
                        self.state
                            .check_lookbehind_group(group, self.source.tell())?;
                        return Ok(Some(Op::GroupRef(group)));
                    } else {
                        let suffix = self.source.get()?.unwrap_or_default();
                        return Err(self.error(
                            format!("unknown extension ?P{suffix}"),
                            suffix.chars().count() + 2,
                        ));
                    }
                }
                ":" => capture = false,
                "#" => {
                    loop {
                        match self.source.get()? {
                            Some(value) if value == ")" => break,
                            Some(_) => {}
                            None => {
                                return Err(Error::at("missing ), unterminated comment", start));
                            }
                        }
                    }
                    return Ok(None);
                }
                "=" | "!" | "<" => {
                    let mut behind = false;
                    if extension == "<" {
                        extension = self
                            .source
                            .get()?
                            .ok_or_else(|| self.error("unexpected end of pattern", 0))?;
                        if extension != "=" && extension != "!" {
                            return Err(self.error(
                                format!("unknown extension ?<{extension}"),
                                extension.chars().count() + 2,
                            ));
                        }
                        behind = true;
                    }
                    let previous_lookbehind = self.state.lookbehind_groups;
                    if behind && previous_lookbehind.is_none() {
                        self.state.lookbehind_groups = Some(self.state.group_widths.len());
                    }
                    let body = self.parse_sub(*verbose, nested + 1)?;
                    if behind && previous_lookbehind.is_none() {
                        self.state.lookbehind_groups = None;
                    }
                    if !self.source.matches(")")? {
                        return Err(Error::at("missing ), unterminated subpattern", start));
                    }
                    return Ok(Some(Op::Assert {
                        negative: extension == "!",
                        behind,
                        body,
                    }));
                }
                "(" => {
                    let condition = self.source.get_until(")", "group name")?;
                    let group = if condition.chars().all(|c| c.is_ascii_digit()) {
                        let group: usize = condition.parse().unwrap_or(usize::MAX);
                        if group == 0 {
                            return Err(self.error("bad group number", condition.len() + 1));
                        }
                        if group >= MAXGROUPS {
                            return Err(self.error(
                                format!("invalid group reference {group}"),
                                condition.len() + 1,
                            ));
                        }
                        self.state
                            .forward_group_refs
                            .entry(group)
                            .or_insert(self.source.tell().saturating_sub(condition.len() + 1));
                        group
                    } else {
                        self.check_group_name(&condition, 1)?;
                        *self.state.group_names.get(&condition).ok_or_else(|| {
                            self.error(
                                format!("unknown group name '{condition}'"),
                                condition.chars().count() + 1,
                            )
                        })?
                    };
                    self.state
                        .check_lookbehind_group(group, self.source.tell())?;
                    let yes = self.parse_sequence(*verbose, nested + 1, false)?;
                    let no = if self.source.matches("|")? {
                        let branch = self.parse_sequence(*verbose, nested + 1, false)?;
                        if self.source.token()?.as_deref() == Some("|") {
                            return Err(
                                self.error("conditional backref with more than two branches", 0)
                            );
                        }
                        Some(branch)
                    } else {
                        None
                    };
                    if !self.source.matches(")")? {
                        return Err(Error::at("missing ), unterminated subpattern", start));
                    }
                    return Ok(Some(Op::GroupRefExists { group, yes, no }));
                }
                ">" => {
                    capture = false;
                    atomic = true;
                }
                value if flag_for(value).is_some() || value == "-" => {
                    let flags = self.parse_flags(value)?;
                    if let Some((add, del)) = flags {
                        add_flags = add;
                        del_flags = del;
                        capture = false;
                    } else {
                        if !first || !prefix.is_empty() {
                            return Err(Error::at(
                                "global flags not at the start of the expression",
                                start,
                            ));
                        }
                        *verbose = self.state.flags & FLAG_VERBOSE != 0;
                        return Ok(None);
                    }
                }
                _ => {
                    return Err(self.error(
                        format!("unknown extension ?{extension}"),
                        extension.chars().count() + 1,
                    ));
                }
            }
        }

        let group = if capture {
            let error_position = name
                .as_ref()
                .map(|value| self.source.tell().saturating_sub(value.chars().count() + 1))
                .unwrap_or(start);
            Some(
                self.state
                    .open_group(name)
                    .map_err(|error| Error::at(error.message, error_position))?,
            )
        } else {
            None
        };
        let sub_verbose =
            (*verbose || add_flags & FLAG_VERBOSE != 0) && del_flags & FLAG_VERBOSE == 0;
        let body = self.parse_sub(sub_verbose, nested + 1)?;
        if !self.source.matches(")")? {
            return Err(Error::at("missing ), unterminated subpattern", start));
        }
        if let Some(group) = group {
            self.state.group_widths[group] = Some(width(&body, &self.state));
        }
        Ok(Some(if atomic {
            Op::Atomic(body)
        } else {
            Op::Subpattern {
                group,
                add_flags,
                del_flags,
                body,
            }
        }))
    }

    fn parse_flags(&mut self, initial: &str) -> Result<Option<(u16, u16)>, Error> {
        let mut token = initial.to_string();
        let mut add = 0;
        let mut del = 0;
        if token != "-" {
            loop {
                let flag = flag_for(&token).expect("caller checked flag");
                if token == "L" {
                    return Err(self.error(
                        "bad inline flags: cannot use 'L' flag with a str pattern",
                        0,
                    ));
                }
                add |= flag;
                if flag & TYPE_FLAGS != 0 && add & TYPE_FLAGS != flag {
                    return Err(self.error(
                        "bad inline flags: flags 'a', 'u' and 'L' are incompatible",
                        0,
                    ));
                }
                token = self
                    .source
                    .get()?
                    .ok_or_else(|| self.error("missing -, : or )", 0))?;
                if matches!(token.as_str(), ")" | "-" | ":") {
                    break;
                }
                if flag_for(&token).is_none() {
                    return Err(self.error(
                        if one_char(&token).is_some_and(char::is_alphabetic) {
                            "unknown flag"
                        } else {
                            "missing -, : or )"
                        },
                        token.chars().count(),
                    ));
                }
            }
        }
        if token == ")" {
            self.state.flags |= add;
            return Ok(None);
        }
        if add & GLOBAL_FLAGS != 0 {
            return Err(self.error("bad inline flags: cannot turn on global flag", 1));
        }
        if token == "-" {
            token = self
                .source
                .get()?
                .ok_or_else(|| self.error("missing flag", 0))?;
            loop {
                let Some(flag) = flag_for(&token) else {
                    return Err(self.error(
                        if one_char(&token).is_some_and(char::is_alphabetic) {
                            "unknown flag"
                        } else {
                            "missing flag"
                        },
                        token.chars().count(),
                    ));
                };
                if flag & TYPE_FLAGS != 0 {
                    return Err(self.error(
                        "bad inline flags: cannot turn off flags 'a', 'u' and 'L'",
                        0,
                    ));
                }
                del |= flag;
                token = self
                    .source
                    .get()?
                    .ok_or_else(|| self.error("missing :", 0))?;
                if token == ":" {
                    break;
                }
            }
        }
        if add & del != 0 {
            return Err(self.error("bad inline flags: flag turned on and off", 1));
        }
        Ok(Some((add, del)))
    }

    fn check_group_name(&self, name: &str, offset: usize) -> Result<(), Error> {
        let mut chars = name.chars();
        let valid = chars
            .next()
            .is_some_and(|ch| ch == '_' || unicode_ident::is_xid_start(ch))
            && chars.all(|ch| ch == '_' || unicode_ident::is_xid_continue(ch));
        if valid {
            Ok(())
        } else {
            Err(self.error(
                format!("bad character in group name '{name}'"),
                name.chars().count() + offset,
            ))
        }
    }

    fn parse_escape(&mut self, token: &str, in_set: bool) -> Result<Op, Error> {
        if let Some(value) = simple_escape(token, in_set) {
            return Ok(value);
        }
        let code = token.chars().nth(1).expect("escape token");
        match code {
            'x' | 'u' | 'U' => {
                let count = match code {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let digits = self.source.get_while(count, is_hex)?;
                let whole = format!("{token}{digits}");
                if digits.len() != count {
                    return Err(self.error(format!("incomplete escape {whole}"), whole.len()));
                }
                let value = u32::from_str_radix(&digits, 16).expect("hex checked");
                if char::from_u32(value).is_none() {
                    return Err(self.error(format!("bad escape {whole}"), whole.len()));
                }
                Ok(Op::Literal(value))
            }
            'N' => {
                if !self.source.matches("{")? {
                    return Err(self.error("missing {", 0));
                }
                let name = self.source.get_until("}", "character name")?;
                let value = unicode_names2::character(&name).ok_or_else(|| {
                    self.error(
                        format!("undefined character name '{name}'"),
                        name.chars().count() + 4,
                    )
                })?;
                Ok(Op::Literal(value as u32))
            }
            '0' if !in_set => {
                let extra = self.source.get_while(2, is_octal)?;
                Ok(Op::Literal(
                    u32::from_str_radix(&format!("0{extra}"), 8).unwrap(),
                ))
            }
            c if c.is_ascii_digit() => self.parse_numeric_escape(token, in_set),
            c if c.is_ascii_alphabetic() => {
                Err(self.error(format!("bad escape {token}"), token.chars().count()))
            }
            c => Ok(Op::Literal(c as u32)),
        }
    }

    fn parse_numeric_escape(&mut self, token: &str, in_set: bool) -> Result<Op, Error> {
        let mut digits = token[1..].to_string();
        if in_set {
            digits.push_str(&self.source.get_while(2, is_octal)?);
            let value = u32::from_str_radix(&digits, 8)
                .map_err(|_| self.error(format!("bad escape \\{digits}"), digits.len() + 1))?;
            if value > 0o377 {
                return Err(self.error(
                    format!("octal escape value \\{digits} outside of range 0-0o377"),
                    digits.len() + 1,
                ));
            }
            return Ok(Op::Literal(value));
        }

        if self
            .source
            .token()?
            .as_deref()
            .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
        {
            digits.push_str(&self.source.get()?.expect("peeked token"));
            if digits.chars().take(2).all(is_octal)
                && self
                    .source
                    .token()?
                    .as_deref()
                    .is_some_and(|s| s.chars().all(is_octal))
            {
                digits.push_str(&self.source.get()?.expect("peeked token"));
                let value = u32::from_str_radix(&digits, 8).unwrap();
                if value > 0o377 {
                    return Err(self.error(
                        format!("octal escape value \\{digits} outside of range 0-0o377"),
                        digits.len() + 1,
                    ));
                }
                return Ok(Op::Literal(value));
            }
        }
        let group: usize = digits.parse().unwrap_or(usize::MAX);
        if group < self.state.group_widths.len() {
            if !self.state.group_is_closed(group) {
                return Err(self.error("cannot refer to an open group", digits.len() + 1));
            }
            self.state
                .check_lookbehind_group(group, self.source.tell())?;
            Ok(Op::GroupRef(group))
        } else {
            Err(self.error(format!("invalid group reference {group}"), digits.len()))
        }
    }
}

fn one_char(token: &str) -> Option<char> {
    let mut chars = token.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn is_digit(ch: char) -> bool {
    ch.is_ascii_digit()
}
fn is_octal(ch: char) -> bool {
    matches!(ch, '0'..='7')
}
fn is_hex(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}
fn is_verbose_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
}

fn parse_repeat_bound(value: &str) -> Result<Option<usize>, Error> {
    if value.is_empty() {
        return Ok(None);
    }
    let value = value
        .parse::<u64>()
        .map_err(|_| Error::plain("the repetition number is too large"))?;
    if value >= MAXREPEAT as u64 {
        Err(Error::plain("the repetition number is too large"))
    } else {
        Ok(Some(value as usize))
    }
}

fn simple_escape(token: &str, in_set: bool) -> Option<Op> {
    let literal = match token {
        "\\a" => Some('\u{7}'),
        "\\b" if in_set => Some('\u{8}'),
        "\\f" => Some('\u{c}'),
        "\\n" => Some('\n'),
        "\\r" => Some('\r'),
        "\\t" => Some('\t'),
        "\\v" => Some('\u{b}'),
        "\\\\" => Some('\\'),
        _ => None,
    };
    if let Some(value) = literal {
        return Some(Op::Literal(value as u32));
    }
    let category = match token {
        "\\d" => Some(SreCatCode::DIGIT as u32),
        "\\D" => Some(SreCatCode::NOT_DIGIT as u32),
        "\\s" => Some(SreCatCode::SPACE as u32),
        "\\S" => Some(SreCatCode::NOT_SPACE as u32),
        "\\w" => Some(SreCatCode::WORD as u32),
        "\\W" => Some(SreCatCode::NOT_WORD as u32),
        _ => None,
    };
    if let Some(category) = category {
        return Some(Op::Set(vec![SetOp::Category(category)]));
    }
    if in_set {
        return None;
    }
    Some(match token {
        "\\A" => Op::At(SreAtCode::BEGINNING_STRING as u32),
        "\\b" => Op::At(SreAtCode::BOUNDARY as u32),
        "\\B" => Op::At(SreAtCode::NON_BOUNDARY as u32),
        "\\Z" => Op::At(SreAtCode::END_STRING as u32),
        _ => return None,
    })
}

fn flag_for(token: &str) -> Option<u16> {
    Some(match token {
        "i" => FLAG_IGNORECASE,
        "L" => FLAG_LOCALE,
        "m" => FLAG_MULTILINE,
        "s" => FLAG_DOTALL,
        "x" => FLAG_VERBOSE,
        "a" => FLAG_ASCII,
        "t" => FLAG_TEMPLATE,
        "u" => FLAG_UNICODE,
        _ => return None,
    })
}

fn fix_flags(mut flags: u16) -> Result<u16, Error> {
    if flags & FLAG_LOCALE != 0 {
        return Err(Error::plain("cannot use LOCALE flag with a str pattern"));
    }
    if flags & FLAG_ASCII == 0 {
        flags |= FLAG_UNICODE;
    } else if flags & FLAG_UNICODE != 0 {
        return Err(Error::plain("ASCII and UNICODE flags are incompatible"));
    }
    Ok(flags)
}

fn dedup_set(items: &mut Vec<SetOp>) {
    let mut unique = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    *items = unique;
}

fn width(pattern: &Pattern, state: &ParseState) -> (u64, u64) {
    let (mut low, mut high) = (0u64, 0u64);
    for op in pattern {
        let (item_low, item_high) = match op {
            Op::Branch(branches) => branches.iter().map(|branch| width(branch, state)).fold(
                (MAX_WIDTH, 0),
                |(low, high), (branch_low, branch_high)| {
                    (low.min(branch_low), high.max(branch_high))
                },
            ),
            Op::Atomic(body) | Op::Subpattern { body, .. } => width(body, state),
            Op::Repeat { min, max, body, .. } => {
                let (body_low, body_high) = width(body, state);
                let low = body_low.saturating_mul(*min as u64);
                let high = if *max == MAXREPEAT && body_high != 0 {
                    MAX_WIDTH
                } else {
                    body_high.saturating_mul(*max as u64)
                };
                (low, high)
            }
            Op::Literal(_) | Op::NotLiteral(_) | Op::Set(_) | Op::Any => (1, 1),
            Op::GroupRef(group) => state.group_widths[*group].unwrap_or((0, MAX_WIDTH)),
            Op::GroupRefExists { yes, no, .. } => {
                let (yes_low, yes_high) = width(yes, state);
                let (no_low, no_high) = no.as_ref().map_or((0, 0), |body| width(body, state));
                (yes_low.min(no_low), yes_high.max(no_high))
            }
            Op::Assert { .. } | Op::At(_) => (0, 0),
        };
        low = low.saturating_add(item_low);
        high = high.saturating_add(item_high);
    }
    (low, high)
}

fn emit(codes: &mut Vec<u32>, opcode: SreOpcode) {
    codes.push(opcode as u32);
}

fn combine_flags(mut flags: u16, add: u16, del: u16) -> u16 {
    if add & TYPE_FLAGS != 0 {
        flags &= !TYPE_FLAGS;
    }
    (flags | add) & !del
}

// Mirrors CPython 3.12's `_get_charset_prefix`: scoped type flags affect the
// real opcode but the INFO search charset is compiled with the outer flags.
// That observable quirk is why `(?a:\W)` search differs from fullmatch.
fn first_charset(pattern: &Pattern) -> Option<&[SetOp]> {
    let first = pattern.first()?;
    match first {
        Op::Set(items) => Some(items),
        Op::Subpattern { body, .. } => first_charset(body),
        _ => None,
    }
}

fn compile_pattern(
    pattern: &Pattern,
    flags: u16,
    state: &ParseState,
    codes: &mut Vec<u32>,
) -> Result<(), Error> {
    for op in pattern {
        match op {
            Op::Literal(value) => compile_literal(*value, false, flags, codes),
            Op::NotLiteral(value) => compile_literal(*value, true, flags, codes),
            Op::Set(items) => compile_set(items, flags, codes),
            Op::Any => emit(
                codes,
                if flags & FLAG_DOTALL != 0 {
                    SreOpcode::ANY_ALL
                } else {
                    SreOpcode::ANY
                },
            ),
            Op::Repeat {
                kind,
                min,
                max,
                body,
            } => {
                let simple = is_simple(body);
                let opcode = match (kind, simple) {
                    (RepeatKind::Greedy, true) => SreOpcode::REPEAT_ONE,
                    (RepeatKind::Lazy, true) => SreOpcode::MIN_REPEAT_ONE,
                    (RepeatKind::Possessive, true) => SreOpcode::POSSESSIVE_REPEAT_ONE,
                    (RepeatKind::Greedy | RepeatKind::Lazy, false) => SreOpcode::REPEAT,
                    (RepeatKind::Possessive, false) => SreOpcode::POSSESSIVE_REPEAT,
                };
                emit(codes, opcode);
                let skip = codes.len();
                codes.extend([0, *min as u32, *max as u32]);
                compile_pattern(body, flags, state, codes)?;
                match (kind, simple) {
                    (RepeatKind::Greedy, false) => {
                        codes[skip] = (codes.len() - skip) as u32;
                        emit(codes, SreOpcode::MAX_UNTIL);
                    }
                    (RepeatKind::Lazy, false) => {
                        codes[skip] = (codes.len() - skip) as u32;
                        emit(codes, SreOpcode::MIN_UNTIL);
                    }
                    _ => {
                        emit(codes, SreOpcode::SUCCESS);
                        codes[skip] = (codes.len() - skip) as u32;
                    }
                }
            }
            Op::Subpattern {
                group,
                add_flags,
                del_flags,
                body,
            } => {
                if let Some(group) = group {
                    emit(codes, SreOpcode::MARK);
                    codes.push(((group - 1) * 2) as u32);
                }
                compile_pattern(
                    body,
                    combine_flags(flags, *add_flags, *del_flags),
                    state,
                    codes,
                )?;
                if let Some(group) = group {
                    emit(codes, SreOpcode::MARK);
                    codes.push(((group - 1) * 2 + 1) as u32);
                }
            }
            Op::Atomic(body) => {
                emit(codes, SreOpcode::ATOMIC_GROUP);
                let skip = codes.len();
                codes.push(0);
                compile_pattern(body, flags, state, codes)?;
                emit(codes, SreOpcode::SUCCESS);
                codes[skip] = (codes.len() - skip) as u32;
            }
            Op::Assert {
                negative,
                behind,
                body,
            } => {
                emit(
                    codes,
                    if *negative {
                        SreOpcode::ASSERT_NOT
                    } else {
                        SreOpcode::ASSERT
                    },
                );
                let skip = codes.len();
                codes.push(0);
                if *behind {
                    let (low, high) = width(body, state);
                    if low != high {
                        return Err(Error::plain("look-behind requires fixed-width pattern"));
                    }
                    codes.push(low as u32);
                } else {
                    codes.push(0);
                }
                compile_pattern(body, flags, state, codes)?;
                emit(codes, SreOpcode::SUCCESS);
                codes[skip] = (codes.len() - skip) as u32;
            }
            Op::At(value) => {
                emit(codes, SreOpcode::AT);
                codes.push(map_at(*value, flags));
            }
            Op::Branch(branches) => {
                emit(codes, SreOpcode::BRANCH);
                let mut tails = Vec::new();
                for branch in branches {
                    let skip = codes.len();
                    codes.push(0);
                    compile_pattern(branch, flags, state, codes)?;
                    emit(codes, SreOpcode::JUMP);
                    tails.push(codes.len());
                    codes.push(0);
                    codes[skip] = (codes.len() - skip) as u32;
                }
                emit(codes, SreOpcode::FAILURE);
                let end = codes.len();
                for tail in tails {
                    codes[tail] = (end - tail) as u32;
                }
            }
            Op::GroupRef(group) => {
                emit(
                    codes,
                    match ignore_mode(flags) {
                        IgnoreMode::None => SreOpcode::GROUPREF,
                        IgnoreMode::Ascii => SreOpcode::GROUPREF_IGNORE,
                        IgnoreMode::Unicode => SreOpcode::GROUPREF_UNI_IGNORE,
                    },
                );
                codes.push((group - 1) as u32);
            }
            Op::GroupRefExists { group, yes, no } => {
                emit(codes, SreOpcode::GROUPREF_EXISTS);
                codes.push((group - 1) as u32);
                let skip_yes = codes.len();
                codes.push(0);
                compile_pattern(yes, flags, state, codes)?;
                if let Some(no) = no {
                    emit(codes, SreOpcode::JUMP);
                    let skip_no = codes.len();
                    codes.push(0);
                    codes[skip_yes] = (codes.len() - skip_yes + 1) as u32;
                    compile_pattern(no, flags, state, codes)?;
                    codes[skip_no] = (codes.len() - skip_no) as u32;
                } else {
                    codes[skip_yes] = (codes.len() - skip_yes + 1) as u32;
                }
            }
        }
    }
    Ok(())
}

fn is_simple(pattern: &Pattern) -> bool {
    if pattern.len() != 1 {
        return false;
    }
    match &pattern[0] {
        Op::Literal(_) | Op::NotLiteral(_) | Op::Set(_) | Op::Any => true,
        Op::Subpattern {
            group: None, body, ..
        } => is_simple(body),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum IgnoreMode {
    None,
    Ascii,
    Unicode,
}

fn ignore_mode(flags: u16) -> IgnoreMode {
    if flags & FLAG_IGNORECASE == 0 {
        IgnoreMode::None
    } else if flags & FLAG_ASCII != 0 {
        IgnoreMode::Ascii
    } else {
        IgnoreMode::Unicode
    }
}

fn compile_literal(value: u32, negate: bool, flags: u16, codes: &mut Vec<u32>) {
    match ignore_mode(flags) {
        IgnoreMode::None => {
            emit(
                codes,
                if negate {
                    SreOpcode::NOT_LITERAL
                } else {
                    SreOpcode::LITERAL
                },
            );
            codes.push(value);
        }
        IgnoreMode::Ascii => {
            emit(
                codes,
                if negate {
                    SreOpcode::NOT_LITERAL_IGNORE
                } else {
                    SreOpcode::LITERAL_IGNORE
                },
            );
            codes.push(crate::string::lower_ascii(value));
        }
        IgnoreMode::Unicode => {
            let lower = crate::string::lower_unicode(value);
            let extras = extra_cases(lower);
            if extras.is_empty() {
                emit(
                    codes,
                    if negate {
                        SreOpcode::NOT_LITERAL_UNI_IGNORE
                    } else {
                        SreOpcode::LITERAL_UNI_IGNORE
                    },
                );
                codes.push(lower);
            } else {
                emit(codes, SreOpcode::IN_UNI_IGNORE);
                let skip = codes.len();
                codes.push(0);
                if negate {
                    emit(codes, SreOpcode::NEGATE);
                }
                for value in core::iter::once(lower).chain(extras.iter().copied()) {
                    emit(codes, SreOpcode::LITERAL);
                    codes.push(value);
                }
                emit(codes, SreOpcode::FAILURE);
                codes[skip] = (codes.len() - skip) as u32;
            }
        }
    }
}

fn compile_set(items: &[SetOp], flags: u16, codes: &mut Vec<u32>) {
    emit(
        codes,
        match ignore_mode(flags) {
            IgnoreMode::None => SreOpcode::IN,
            IgnoreMode::Ascii => SreOpcode::IN_IGNORE,
            IgnoreMode::Unicode => SreOpcode::IN_UNI_IGNORE,
        },
    );
    let skip = codes.len();
    codes.push(0);
    for item in items {
        match item {
            SetOp::Negate => emit(codes, SreOpcode::NEGATE),
            SetOp::Literal(value) => {
                let lower = match ignore_mode(flags) {
                    IgnoreMode::None => *value,
                    IgnoreMode::Ascii => crate::string::lower_ascii(*value),
                    IgnoreMode::Unicode => crate::string::lower_unicode(*value),
                };
                emit(codes, SreOpcode::LITERAL);
                codes.push(lower);
                if matches!(ignore_mode(flags), IgnoreMode::Unicode) {
                    for extra in extra_cases(lower) {
                        emit(codes, SreOpcode::LITERAL);
                        codes.push(*extra);
                    }
                }
            }
            SetOp::Range(low, high) => {
                emit(
                    codes,
                    if matches!(ignore_mode(flags), IgnoreMode::Unicode) {
                        SreOpcode::RANGE_UNI_IGNORE
                    } else {
                        SreOpcode::RANGE
                    },
                );
                codes.extend([*low, *high]);
            }
            SetOp::Category(value) => {
                emit(codes, SreOpcode::CATEGORY);
                codes.push(map_category(*value, flags));
            }
        }
    }
    emit(codes, SreOpcode::FAILURE);
    codes[skip] = (codes.len() - skip) as u32;
}

fn map_at(value: u32, flags: u16) -> u32 {
    if flags & FLAG_MULTILINE != 0 {
        if value == SreAtCode::BEGINNING as u32 {
            return SreAtCode::BEGINNING_LINE as u32;
        }
        if value == SreAtCode::END as u32 {
            return SreAtCode::END_LINE as u32;
        }
    }
    if flags & FLAG_UNICODE != 0 {
        if value == SreAtCode::BOUNDARY as u32 {
            return SreAtCode::UNI_BOUNDARY as u32;
        }
        if value == SreAtCode::NON_BOUNDARY as u32 {
            return SreAtCode::UNI_NON_BOUNDARY as u32;
        }
    }
    value
}

fn map_category(value: u32, flags: u16) -> u32 {
    if flags & FLAG_UNICODE == 0 {
        return value;
    }
    match value {
        x if x == SreCatCode::DIGIT as u32 => SreCatCode::UNI_DIGIT as u32,
        x if x == SreCatCode::NOT_DIGIT as u32 => SreCatCode::UNI_NOT_DIGIT as u32,
        x if x == SreCatCode::SPACE as u32 => SreCatCode::UNI_SPACE as u32,
        x if x == SreCatCode::NOT_SPACE as u32 => SreCatCode::UNI_NOT_SPACE as u32,
        x if x == SreCatCode::WORD as u32 => SreCatCode::UNI_WORD as u32,
        x if x == SreCatCode::NOT_WORD as u32 => SreCatCode::UNI_NOT_WORD as u32,
        x if x == SreCatCode::LINEBREAK as u32 => SreCatCode::UNI_LINEBREAK as u32,
        x if x == SreCatCode::NOT_LINEBREAK as u32 => SreCatCode::UNI_NOT_LINEBREAK as u32,
        _ => value,
    }
}

fn extra_cases(value: u32) -> &'static [u32] {
    match value {
        0x0069 => &[0x0131],
        0x0073 => &[0x017f],
        0x00b5 => &[0x03bc],
        0x0131 => &[0x0069],
        0x017f => &[0x0073],
        0x0345 => &[0x03b9, 0x1fbe],
        0x0390 => &[0x1fd3],
        0x03b0 => &[0x1fe3],
        0x03b2 => &[0x03d0],
        0x03b5 => &[0x03f5],
        0x03b8 => &[0x03d1],
        0x03b9 => &[0x0345, 0x1fbe],
        0x03ba => &[0x03f0],
        0x03bc => &[0x00b5],
        0x03c0 => &[0x03d6],
        0x03c1 => &[0x03f1],
        0x03c2 => &[0x03c3],
        0x03c3 => &[0x03c2],
        0x03c6 => &[0x03d5],
        0x03d0 => &[0x03b2],
        0x03d1 => &[0x03b8],
        0x03d5 => &[0x03c6],
        0x03d6 => &[0x03c0],
        0x03f0 => &[0x03ba],
        0x03f1 => &[0x03c1],
        0x03f5 => &[0x03b5],
        0x0432 => &[0x1c80],
        0x0434 => &[0x1c81],
        0x043e => &[0x1c82],
        0x0441 => &[0x1c83],
        0x0442 => &[0x1c84, 0x1c85],
        0x044a => &[0x1c86],
        0x0463 => &[0x1c87],
        0x1c80 => &[0x0432],
        0x1c81 => &[0x0434],
        0x1c82 => &[0x043e],
        0x1c83 => &[0x0441],
        0x1c84 => &[0x0442, 0x1c85],
        0x1c85 => &[0x0442, 0x1c84],
        0x1c86 => &[0x044a],
        0x1c87 => &[0x0463],
        0x1c88 => &[0xa64b],
        0x1e61 => &[0x1e9b],
        0x1e9b => &[0x1e61],
        0x1fbe => &[0x0345, 0x03b9],
        0x1fd3 => &[0x0390],
        0x1fe3 => &[0x03b0],
        0xa64b => &[0x1c88],
        0xfb05 => &[0xfb06],
        0xfb06 => &[0xfb05],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Request, SearchIter, State, StrDrive};

    fn findall(pattern: &str, text: &str, ignore_case: bool) -> Vec<Vec<String>> {
        let compiled = compile(pattern, ignore_case).unwrap();
        let req = Request::new(text, 0, text.count(), &compiled.codes, false);
        let mut iter = SearchIter {
            req,
            state: State::default(),
        };
        let chars: Vec<char> = text.chars().collect();
        let mut matches = Vec::new();
        while iter.next().is_some() {
            let mut groups = Vec::new();
            if compiled.groups == 0 {
                groups.push(
                    chars[iter.state.start..iter.state.cursor.position]
                        .iter()
                        .collect(),
                );
            } else {
                for group in 0..compiled.groups {
                    let (start, end) = iter.state.marks.get(group);
                    groups.push(match (start.into_option(), end.into_option()) {
                        (Some(start), Some(end)) => chars[start..end].iter().collect(),
                        _ => String::new(),
                    });
                }
            }
            matches.push(groups);
        }
        matches
    }

    #[test]
    fn compiles_cpython_312_constructs() {
        for pattern in [
            r"\d+",
            r"(a)|(b)",
            r"apple(?= pie)",
            r"(?P<x>a)(?P=x)",
            r"(?<=abc)def",
            r"(?>x)++x",
            r"(a)?(?(1)b|c)",
            r"\N{EM DASH}",
        ] {
            let compiled = compile(pattern, pattern.contains("apple")).unwrap();
            assert_eq!(compiled.codes[0], SreOpcode::INFO as u32);
            assert_eq!(*compiled.codes.last().unwrap(), SreOpcode::SUCCESS as u32);
        }
    }

    #[test]
    fn maps_cpython_312_errors() {
        for (pattern, error) in [
            ("(", "missing ), unterminated subpattern at position 0"),
            ("[", "unterminated character set at position 0"),
            ("*", "nothing to repeat at position 0"),
            (r"\x", r"incomplete escape \x at position 0"),
            (
                "a(?i)b",
                "global flags not at the start of the expression at position 1",
            ),
            ("(?<=a*)b", "look-behind requires fixed-width pattern"),
            (r"\1", "invalid group reference 1 at position 1"),
            ("[z-a]", "bad character range z-a at position 1"),
            ("a{4,2}", "min repeat greater than max repeat at position 2"),
        ] {
            assert_eq!(
                compile(pattern, false).unwrap_err().to_string(),
                error,
                "{pattern}"
            );
        }
    }

    #[test]
    fn executes_cpython_312_findall_semantics() {
        assert_eq!(
            findall(r"\d+", "price 42 then 99", false),
            vec![vec![String::from("42")], vec![String::from("99")]]
        );
        assert_eq!(
            findall(r"(a)|(b)", "ab", false),
            vec![
                vec![String::from("a"), String::new()],
                vec![String::new(), String::from("b")]
            ]
        );
        assert_eq!(
            findall(r"apple(?= pie)", "APPLE PIE apple tart", true),
            vec![vec![String::from("APPLE")]]
        );
        assert_eq!(
            findall(r"(?P<x>a)(?P=x)", "zaa", false),
            vec![vec![String::from("a")]]
        );
        assert_eq!(
            findall(r"price (\d+)", "price 42 then price 99", false),
            vec![vec![String::from("42")], vec![String::from("99")]]
        );
        assert_eq!(
            findall(r"(\w+)=(\d+)", "a=1 b=2", false),
            vec![
                vec![String::from("a"), String::from("1")],
                vec![String::from("b"), String::from("2")]
            ]
        );
        assert_eq!(
            findall(r"(?<=abc)def", "abcdef", false),
            vec![vec![String::from("def")]]
        );
        assert_eq!(
            findall(r"\N{EM DASH}", "x—y", false),
            vec![vec![String::from("—")]]
        );
    }

    #[test]
    fn minimized_cpython_312_differential_regressions() {
        assert_eq!(findall(r"\d", "٣", false), vec![vec![String::from("٣")]]);
        assert_eq!(
            findall("[a-z]", "İıſK", true),
            vec![
                vec![String::from("İ")],
                vec![String::from("ı")],
                vec![String::from("ſ")],
                vec![String::from("K")],
            ]
        );

        // CPython 3.12 compiles a scoped ASCII set correctly for matching,
        // but its INFO search charset uses the outer Unicode flags.
        assert!(findall(r"(?a:\W)", "Ω", false).is_empty());

        assert_eq!(
            findall(r"(?:a?)*", "a", false),
            vec![vec![String::from("a")], vec![String::new()]]
        );
        assert_eq!(
            findall(r"(?=(a+))", "aa", false),
            vec![vec![String::from("aa")], vec![String::from("a")]]
        );
        assert!(findall(r"a*+a", "aaa", false).is_empty());

        assert_eq!(
            compile("(?P<x>a)(?P<x>b)", false).unwrap_err().to_string(),
            "redefinition of group name 'x' as group 2; was group 1 at position 12"
        );
    }
}
