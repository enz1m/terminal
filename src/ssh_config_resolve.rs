use super::*;

pub(super) fn has_meta(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'*' | b'?' | b'['))
}

fn atom(
    pattern: &[u8],
    at: usize,
    byte: u8,
    budget: &mut Budget<'_>,
    fold_case: bool,
) -> Result<Option<usize>> {
    budget.charge(BudgetKind::ResolveWork, 1)?;
    let fold = |b: u8| if fold_case { b.to_ascii_lowercase() } else { b };
    match pattern[at] {
        b'?' => Ok(Some(at + 1)),
        b'[' => {
            let mut end = at + 1;
            while end < pattern.len() && pattern[end] != b']' {
                budget.charge(BudgetKind::ResolveWork, 1)?;
                end += 1;
            }
            if end == pattern.len() {
                return Ok((byte == b'[').then_some(at + 1));
            }
            let mut index = at + 1;
            let negated = matches!(pattern.get(index), Some(b'!' | b'^'));
            if negated {
                index += 1;
            }
            let byte = fold(byte);
            let mut hit = false;
            while index < end {
                budget.charge(BudgetKind::ResolveWork, 1)?;
                let start = fold(pattern[index]);
                if index + 2 < end && pattern[index + 1] == b'-' {
                    hit |= start <= byte && byte <= fold(pattern[index + 2]);
                    index += 3;
                } else {
                    hit |= start == byte;
                    index += 1;
                }
            }
            Ok((hit != negated).then_some(end + 1))
        }
        literal => Ok((fold(literal) == fold(byte)).then_some(at + 1)),
    }
}
pub(super) fn wildcard(pattern: &str, value: &str, budget: &mut Budget<'_>) -> Result<bool> {
    match_pattern(pattern, value, budget, true)
}
pub(super) fn glob(pattern: &str, value: &str, budget: &mut Budget<'_>) -> Result<bool> {
    match_pattern(pattern, value, budget, false)
}
fn match_pattern(
    pattern: &str,
    value: &str,
    budget: &mut Budget<'_>,
    fold_case: bool,
) -> Result<bool> {
    let (pattern, value) = (pattern.as_bytes(), value.as_bytes());
    let (mut p, mut v, mut star) = (0, 0, None);
    while v < value.len() {
        budget.charge(BudgetKind::ResolveWork, 1)?;
        if pattern.get(p) == Some(&b'*') {
            star = Some((p + 1, v));
            p += 1;
        } else if p < pattern.len() {
            if let Some(next) = atom(pattern, p, value[v], budget, fold_case)? {
                p = next;
                v += 1;
                continue;
            }
            if let Some((restart, used)) = star {
                star = Some((restart, used + 1));
                p = restart;
                v = used + 1;
            } else {
                return Ok(false);
            }
        } else if let Some((restart, used)) = star {
            star = Some((restart, used + 1));
            p = restart;
            v = used + 1;
        } else {
            return Ok(false);
        }
    }
    while pattern.get(p) == Some(&b'*') {
        budget.charge(BudgetKind::ResolveWork, 1)?;
        p += 1;
    }
    Ok(p == pattern.len())
}

fn assign(
    index: usize,
    values: &Values,
    resolved: &mut [Values],
    remaining: &mut [usize; 4],
    budget: &mut Budget<'_>,
) -> Result<()> {
    for (field, value) in values.text.iter().enumerate() {
        budget.charge(BudgetKind::ResolveWork, 1)?;
        if let Some(value) = value
            && resolved[index].text[field].is_none()
        {
            budget.charge(BudgetKind::ResolveWork, value.len())?;
            resolved[index].text[field] = Some(budget.owned(value)?);
            remaining[field] -= 1;
        }
    }
    Ok(())
}
pub(super) fn hosts(parser: Parser, budget: &mut Budget<'_>) -> Result<Vec<OpenSshHost>> {
    let count = parser.names.len();
    budget.charge(
        BudgetKind::RetainedBytes,
        count.saturating_mul(std::mem::size_of::<Values>() + std::mem::size_of::<usize>()),
    )?;
    let mut resolved: Vec<Values> = (0..count).map(|_| Values::default()).collect();
    let mut remaining = [count; 4];
    let mut visited = vec![0_usize; count];
    for (ordinal, block) in parser.blocks.iter().enumerate() {
        budget.charge(BudgetKind::ResolveWork, 1)?;
        if block
            .values
            .text
            .iter()
            .enumerate()
            .all(|(field, value)| value.is_none() || remaining[field] == 0)
        {
            continue;
        }
        if block.patterns.iter().any(|p| p == "*") && block.negatives.is_empty() {
            // A broad default exhausts its field permanently: at most four full
            // alias scans are needed, regardless of the number of default blocks.
            for (field, value) in block.values.text.iter().enumerate() {
                if let Some(value) = value {
                    if remaining[field] == 0 {
                        continue;
                    }
                    for item in &mut resolved {
                        budget.charge(BudgetKind::ResolveWork, 1)?;
                        if item.text[field].is_none() {
                            budget.charge(BudgetKind::ResolveWork, value.len())?;
                            item.text[field] = Some(budget.owned(value)?);
                        }
                    }
                    remaining[field] = 0;
                }
            }
            continue;
        }
        for pattern in &block.patterns {
            budget.charge(
                BudgetKind::ResolveWork,
                (1 + count.checked_ilog2().unwrap_or(0) as usize)
                    .saturating_mul(pattern.len().max(1)),
            )?;
            let prefix_end = pattern
                .bytes()
                .position(|b| matches!(b, b'*' | b'?' | b'['))
                .unwrap_or(pattern.len());
            let prefix = &pattern[..prefix_end];
            let upper = if has_meta(pattern) {
                std::ops::Bound::Unbounded
            } else {
                std::ops::Bound::Included(pattern.as_str())
            };
            let candidates = parser
                .aliases
                .range::<str, _>((std::ops::Bound::Included(prefix), upper))
                .take_while(move |(key, _)| key.starts_with(prefix));
            for (alias, &index) in candidates {
                budget.charge(BudgetKind::ResolveWork, 1 + prefix.len())?;
                if visited[index] == ordinal + 1 {
                    continue;
                }
                if !wildcard(pattern, alias, budget)? {
                    continue;
                }
                visited[index] = ordinal + 1;
                let mut excluded = false;
                for negative in block.negatives.iter().filter_map(|p| p.strip_prefix('!')) {
                    if wildcard(negative, alias, budget)? {
                        excluded = true;
                        break;
                    }
                }
                if !excluded {
                    assign(index, &block.values, &mut resolved, &mut remaining, budget)?;
                }
            }
        }
    }
    let fingerprint = parser.fingerprint.finish();
    let mut hosts = Vec::new();
    for (alias, values) in parser.names.into_iter().zip(resolved) {
        budget.charge(BudgetKind::ResolveWork, 1)?;
        let [hostname, user, port, identity_file] = values.text;
        let hostname = match hostname {
            Some(name) => name,
            None => {
                budget.charge(BudgetKind::ResolveWork, alias.len())?;
                budget.owned(&alias)?
            }
        };
        budget.push(
            &mut hosts,
            OpenSshHost {
                config_fingerprint: fingerprint,
                alias,
                hostname,
                user: user.unwrap_or_default(),
                port: port.and_then(|port| port.parse().ok()).unwrap_or(22),
                identity_file,
            },
        )?;
    }
    Ok(hosts)
}
