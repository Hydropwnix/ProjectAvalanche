#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Char(char),
    Any,
    Star,
    Set(Vec<(char, char)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    segments: Vec<Option<Vec<Token>>>,
}

#[derive(Debug)]
pub struct Budget(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the patterns took more than the matching budget")]
pub struct Exhausted;

impl Budget {
    fn spend(&mut self, n: u64) -> Result<(), Exhausted> {
        self.0 = self.0.checked_sub(n).ok_or(Exhausted)?;
        Ok(())
    }
}

fn segment(text: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(match c {
            '*' => Token::Star,
            '?' => Token::Any,
            '[' => {
                let mut set = Vec::new();
                loop {
                    match chars.next() {
                        Some(']') if !set.is_empty() => break,
                        Some(']') | None => {
                            return Err(format!("{}: an empty or unclosed `[` set", ssx_core::text::quoted(text)));
                        }
                        Some(lo) if chars.peek() == Some(&'-') => {
                            chars.next();
                            match chars.next() {
                                Some(hi) if hi != ']' && lo <= hi => set.push((lo, hi)),
                                _ => {
                                    return Err(format!("{}: a bad range in a `[` set", ssx_core::text::quoted(text)));
                                }
                            }
                        }
                        Some(one) => set.push((one, one)),
                    }
                }
                Token::Set(set)
            }
            ']' => return Err(format!("{}: a `]` without its `[`", ssx_core::text::quoted(text))),
            c => Token::Char(c),
        });
    }
    Ok(out)
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Self, String> {
        let segments = text
            .split('/')
            .map(|s| if s == "**" { Ok(None) } else { segment(s).map(Some) })
            .collect::<Result<_, _>>()?;
        Ok(Pattern { segments })
    }

    pub fn matches(&self, path: &str, budget: &mut Budget) -> Result<bool, Exhausted> {
        let parts: Vec<&str> = path.split('/').collect();
        let mut failed = vec![false; (self.segments.len() + 1) * (parts.len() + 1)];
        self.from(0, &parts, 0, &mut failed, budget)
    }

    fn from(
        &self,
        i: usize,
        parts: &[&str],
        j: usize,
        failed: &mut [bool],
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        budget.spend(1)?;
        let key = i * (parts.len() + 1) + j;
        if failed[key] {
            return Ok(false);
        }
        let hit = match self.segments.get(i) {
            None => j == parts.len(),
            Some(None) => {
                let mut any = false;
                for k in j..=parts.len() {
                    if self.from(i + 1, parts, k, failed, budget)? {
                        any = true;
                        break;
                    }
                }
                any
            }
            Some(Some(tokens)) => {
                j < parts.len()
                    && seg_match(tokens, parts[j], budget)?
                    && self.from(i + 1, parts, j + 1, failed, budget)?
            }
        };
        if !hit {
            failed[key] = true;
        }
        Ok(hit)
    }
}

fn one(t: &Token, c: char) -> bool {
    match t {
        Token::Char(x) => *x == c,
        Token::Any => true,
        Token::Set(set) => set.iter().any(|&(lo, hi)| (lo..=hi).contains(&c)),
        Token::Star => false,
    }
}

fn seg_match(tokens: &[Token], text: &str, budget: &mut Budget) -> Result<bool, Exhausted> {
    let chars: Vec<char> = text.chars().collect();
    let (mut t, mut c) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while c < chars.len() {
        budget.spend(1)?;
        if t < tokens.len() && tokens[t] == Token::Star {
            star = Some((t, c));
            t += 1;
        } else if t < tokens.len() && one(&tokens[t], chars[c]) {
            (t, c) = (t + 1, c + 1);
        } else if let Some((st, sc)) = star {
            (t, c) = (st + 1, sc + 1);
            star = Some((st, sc + 1));
        } else {
            return Ok(false);
        }
    }
    Ok(tokens[t..].iter().all(|x| *x == Token::Star))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, path: &str) -> bool {
        Pattern::parse(p).unwrap().matches(path, &mut Budget(1 << 20)).unwrap()
    }

    #[test]
    fn segments_stars_and_sets() {
        assert!(m("client/*.luau", "client/hud.luau"));
        assert!(!m("client/*.luau", "client/ui/hud.luau"), "* stays in its folder");
        assert!(m("client/**/*.luau", "client/hud.luau") && m("client/**/*.luau", "client/a/b/hud.luau"));
        assert!(m("**", "a/b/c"));
        assert!(m("ui/icon_[0-9].png", "ui/icon_7.png") && !m("ui/icon_[0-9].png", "ui/icon_x.png"));
        assert!(m("a?c", "abc") && !m("a?c", "ac"));
        assert!(!m("A.png", "a.png"), "case-sensitive, as package paths");
        for bad in ["a[", "a[]", "a]", "[z-a]"] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hostile_patterns_stop_at_the_budget() {
        let star = format!("*{}b", "a".repeat(100));
        let text = "a".repeat(127);
        let p = Pattern::parse(&star).unwrap();
        assert_eq!(p.matches(&text, &mut Budget(1_000)), Err(Exhausted));
        assert_eq!(p.matches(&text, &mut Budget(1 << 20)), Ok(false), "bounded by tokens × characters");
        let deep = Pattern::parse(&vec!["**"; 31].join("/")).unwrap();
        let path = vec!["x"; 32].join("/");
        assert!(deep.matches(&path, &mut Budget(1 << 20)).unwrap(), "memoised: quadratic, not exponential");
    }
}
