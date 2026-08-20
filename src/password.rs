use aws_sdk_cognitoidentityprovider::types::PasswordPolicyType;

const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
/// A subset of the symbols Cognito accepts, leaving out the quoting characters
/// that get mangled when a password is pasted into a shell or a spreadsheet.
const SYMBOLS: &[u8] = b"!#$%&*+-=?@^_";

/// Longer than any default policy asks for, still short enough to read aloud.
const TARGET_LENGTH: usize = 20;
/// Cognito rejects a temporary password longer than this.
const MAX_LENGTH: usize = 256;

/// The pool's password complexity requirements, as far as generating a
/// password needs to care about them.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub minimum_length: usize,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_numbers: bool,
    pub require_symbols: bool,
}

impl Default for Policy {
    /// What Cognito applies when a pool does not spell out a policy.
    fn default() -> Self {
        Self {
            minimum_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_symbols: true,
        }
    }
}

impl From<&PasswordPolicyType> for Policy {
    fn from(policy: &PasswordPolicyType) -> Self {
        let default = Self::default();
        Self {
            minimum_length: policy
                .minimum_length()
                .and_then(|length| usize::try_from(length).ok())
                .unwrap_or(default.minimum_length),
            require_uppercase: policy.require_uppercase(),
            require_lowercase: policy.require_lowercase(),
            require_numbers: policy.require_numbers(),
            require_symbols: policy.require_symbols(),
        }
    }
}

/// A random temporary password that satisfies `policy`.
///
/// Cognito can generate one itself, but only when the new user has an email
/// address or a phone number it could deliver it to; generating it here keeps
/// "leave it blank" working for every pool.
pub fn generate(policy: &Policy) -> Result<String, getrandom::Error> {
    let required: Vec<&[u8]> = [
        (policy.require_lowercase, LOWER),
        (policy.require_uppercase, UPPER),
        (policy.require_numbers, DIGITS),
        (policy.require_symbols, SYMBOLS),
    ]
    .into_iter()
    .filter_map(|(needed, set)| needed.then_some(set))
    .collect();

    // Every class stays in the alphabet even when the policy does not demand
    // it: a policy states minimums, never that a character is forbidden.
    let alphabet: Vec<u8> = [LOWER, UPPER, DIGITS, SYMBOLS].concat();

    let length = policy.minimum_length.clamp(TARGET_LENGTH, MAX_LENGTH);

    let mut password = Vec::with_capacity(length);
    for set in &required {
        password.push(pick(set)?);
    }
    while password.len() < length {
        password.push(pick(&alphabet)?);
    }
    shuffle(&mut password)?;

    Ok(password.iter().map(|&byte| byte as char).collect())
}

/// `alphabet` is always one of the constants above, so the index `below`
/// returns is in range.
fn pick(alphabet: &[u8]) -> Result<u8, getrandom::Error> {
    Ok(alphabet[below(alphabet.len())?])
}

/// Fisher-Yates, so the required characters do not sit in a fixed order.
fn shuffle(bytes: &mut [u8]) -> Result<(), getrandom::Error> {
    for index in (1..bytes.len()).rev() {
        bytes.swap(index, below(index + 1)?);
    }
    Ok(())
}

/// A uniform value below `bound`, which must be positive and well under 2^32.
fn below(bound: usize) -> Result<usize, getrandom::Error> {
    // No caller passes zero; clamping keeps the modulo below from dividing by
    // it even so, since a panic here would take the whole process down.
    let bound = bound.max(1) as u64;
    // Draws landing in the partial final block are rejected, otherwise the
    // low values would come up slightly more often than the high ones.
    let limit = (1u64 << 32) - ((1u64 << 32) % bound);
    loop {
        let mut buf = [0u8; 4];
        getrandom::fill(&mut buf)?;
        let value = u64::from(u32::from_le_bytes(buf));
        if value < limit {
            return Ok((value % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(minimum_length: usize) -> Policy {
        Policy {
            minimum_length,
            ..Policy::default()
        }
    }

    #[test]
    fn satisfies_every_required_class() {
        for _ in 0..64 {
            let password = generate(&policy(8)).unwrap();
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(
                password.chars().any(|c| SYMBOLS.contains(&(c as u8))),
                "no symbol in {password}"
            );
        }
    }

    #[test]
    fn honours_a_minimum_longer_than_the_target() {
        let password = generate(&policy(64)).unwrap();
        assert_eq!(password.chars().count(), 64);
    }

    #[test]
    fn stays_within_the_cognito_maximum() {
        let password = generate(&policy(1000)).unwrap();
        assert_eq!(password.chars().count(), MAX_LENGTH);
    }

    #[test]
    fn never_contains_whitespace() {
        // Cognito's TemporaryPassword pattern is [\S]+.
        for _ in 0..64 {
            let password = generate(&Policy::default()).unwrap();
            assert!(!password.chars().any(char::is_whitespace));
        }
    }

    #[test]
    fn a_policy_without_requirements_still_generates() {
        let relaxed = Policy {
            minimum_length: 6,
            require_uppercase: false,
            require_lowercase: false,
            require_numbers: false,
            require_symbols: false,
        };
        assert_eq!(generate(&relaxed).unwrap().chars().count(), TARGET_LENGTH);
    }

    #[test]
    fn draws_every_value_below_the_bound() {
        let mut seen = [false; 5];
        for _ in 0..500 {
            seen[below(5).unwrap()] = true;
        }
        assert!(seen.iter().all(|&hit| hit));
    }
}
