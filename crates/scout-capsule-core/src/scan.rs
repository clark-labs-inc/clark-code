use crate::{CapsuleError, CapsuleLimits, CapsuleResult};

pub(crate) fn scan_json(input: &[u8], limits: CapsuleLimits) -> CapsuleResult<()> {
    if input.len() > limits.max_input_bytes {
        return Err(CapsuleError::limit(
            "input_bytes",
            limits.max_input_bytes,
            input.len(),
        ));
    }

    let mut depth = 0usize;
    let mut structural_tokens = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut string_token_bytes = 0usize;

    for &byte in input {
        if in_string {
            string_token_bytes += 1;
            if string_token_bytes > limits.max_string_token_bytes {
                return Err(CapsuleError::limit(
                    "string_token_bytes",
                    limits.max_string_token_bytes,
                    string_token_bytes,
                ));
            }
            if escaped {
                escaped = false;
            } else {
                match byte {
                    b'\\' => escaped = true,
                    b'"' => in_string = false,
                    _ => {}
                }
            }
            continue;
        }

        match byte {
            b'"' => {
                in_string = true;
                string_token_bytes = 0;
            }
            b'{' | b'[' => {
                structural_tokens += 1;
                depth += 1;
                if depth > limits.max_nesting_depth {
                    return Err(CapsuleError::limit(
                        "nesting_depth",
                        limits.max_nesting_depth,
                        depth,
                    ));
                }
            }
            b'}' | b']' => {
                structural_tokens += 1;
                depth = depth.saturating_sub(1);
            }
            b',' | b':' => structural_tokens += 1,
            _ => {}
        }
        if structural_tokens > limits.max_structural_tokens {
            return Err(CapsuleError::limit(
                "structural_tokens",
                limits.max_structural_tokens,
                structural_tokens,
            ));
        }
    }
    Ok(())
}
