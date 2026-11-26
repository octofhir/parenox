//! Regex literal spelling and statically proven delimiter errors.
use super::{Kind, OmittedDispatch, Position, ReadError, Reader, Span};

enum RegexEscape {
    None,
    Character,
    Control,
}

impl Reader<'_> {
    pub(super) fn regex(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // "
        let body_start = self.pos.offset;
        let mut complex = false;
        let mut open_groups = 0_usize;
        let mut unmatched_group_close = false;
        let mut saw_opening_bracket = false;
        let mut saw_closing_bracket = false;
        let mut saw_parenthesis = false;
        let mut groups = 0_usize;
        let mut captures_unchecked = false;
        let mut escaped = false;
        let mut regex_escape = RegexEscape::None;
        loop {
            let Some(ch) = self.bump() else {
                return Err(self.error(start, "Unterminated regex literal"));
            };
            if ch == '"' && !escaped {
                break;
            }
            captures_unchecked |= escaped && ch == 'Q';
            escaped = !escaped && ch == '\\';
            complex |= escaped;
            saw_parenthesis |= ch == '(';

            // Java's \c consumes an extra character. Clojure still uses
            // ordinary backslash pairs to decide where the literal ends.
            match regex_escape {
                RegexEscape::Character if ch == 'c' => {
                    regex_escape = RegexEscape::Control;
                    continue;
                }
                RegexEscape::Character | RegexEscape::Control => {
                    regex_escape = RegexEscape::None;
                    continue;
                }
                RegexEscape::None => {}
            }

            match ch {
                '\\' => regex_escape = RegexEscape::Character,
                '(' => {
                    let special_group = self.peek() == Some('?');
                    complex |= special_group;
                    if !special_group || !noncapturing_group(&self.source[self.pos.offset + 1..]) {
                        captures_unchecked |= special_group;
                        groups += 1;
                    }
                    open_groups += 1;
                }
                ')' if open_groups == 0 => unmatched_group_close = true,
                ')' => open_groups -= 1,
                '[' => saw_opening_bracket = true,
                ']' => saw_closing_bracket = true,
                _ => {}
            }
        }
        // Escapes and special groups can quote delimiters or turn on comment mode.
        let error = if complex {
            None
        } else if saw_opening_bracket {
            // A closing bracket can be literal inside a class.
            (!saw_closing_bracket).then_some("Unclosed character class in regex literal")
        } else if unmatched_group_close {
            Some("Unmatched closing group in regex literal")
        } else {
            (open_groups != 0).then_some("Unclosed group in regex literal")
        };
        if let Some(error) = error {
            return Err(self.error(start, error));
        }
        if let Some(dispatches) = &mut self.unchecked_dispatches {
            dispatches.regexes.push(start.offset);
            self.observe_dispatch(
                OmittedDispatch::Regex(Span {
                    start,
                    end: self.pos,
                }),
                self.reading_discard || self.inactive_branch,
            );
        }
        let captures = if !saw_parenthesis {
            Some(0)
        } else if !captures_unchecked
            && (groups == 0 || !saw_opening_bracket)
            && matches!(regex_escape, RegexEscape::None)
        {
            u16::try_from(groups).ok()
        } else {
            None
        };
        let shape = saw_parenthesis
            .then(|| capture_participation(&self.source[body_start..self.pos.offset - 1]))
            .flatten();
        let (captures, required_captures) =
            shape.map_or((captures, 0), |(count, required)| (Some(count), required));
        Ok(Kind::Regex {
            captures,
            required_captures,
        })
    }
}

fn capture_participation(pattern: &str) -> Option<(u16, u32)> {
    let mut chars = pattern.char_indices().peekable();
    let mut count = 0_u16;
    let mut required = 0_u32;
    let mut depth = 0_usize;
    let mut top_level_group = None;
    let mut root_alternation = false;

    while let Some((offset, ch)) = chars.next() {
        match ch {
            '\\' => regex_escape(&mut chars)?,
            '[' => {
                chars.next_if(|(_, ch)| *ch == '^');
                if chars.peek().is_some_and(|(_, ch)| *ch == ']') {
                    // Java treats a leading closing bracket as literal.
                    chars.next();
                }
                loop {
                    match chars.next()?.1 {
                        ']' => break,
                        '[' => return None, // Nested/intersected classes.
                        '\\' => regex_escape(&mut chars)?,
                        _ => {}
                    }
                }
            }
            '(' => {
                if chars.peek().is_some_and(|(_, ch)| *ch == '?') {
                    if !noncapturing_group(&pattern[offset + 2..]) {
                        return None;
                    }
                } else {
                    if depth == 0 {
                        top_level_group = Some(count);
                    }
                    count = count.checked_add(1)?;
                }
                depth += 1;
            }
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0
                    && let Some(group) = top_level_group.take()
                    && !chars
                        .peek()
                        .is_some_and(|(_, ch)| matches!(ch, '?' | '*' | '{'))
                {
                    required |= 1_u32.checked_shl(u32::from(group)).unwrap_or(0);
                }
            }
            '|' if depth == 0 => root_alternation = true,
            _ => {}
        }
    }
    (depth == 0).then_some((count, if root_alternation { 0 } else { required }))
}

fn regex_escape(chars: &mut impl Iterator<Item = (usize, char)>) -> Option<()> {
    match chars.next()?.1 {
        'c' => {
            chars.next()?;
        }
        'Q' | 'N' | 'k' | 'p' | 'P' => return None,
        _ => {}
    }
    Some(())
}

fn noncapturing_group(suffix: &str) -> bool {
    if matches!(
        suffix.as_bytes(),
        [b':' | b'=' | b'!' | b'>', ..] | [b'<', b'=' | b'!', ..]
    ) {
        return true;
    }
    // The x flag changes how the rest of the pattern is parsed.
    let following = suffix.trim_start_matches(['d', 'i', 'm', 's', 'u', 'U']);
    following.len() < suffix.len() && following.starts_with([')', ':'])
}
