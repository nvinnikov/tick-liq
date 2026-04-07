use anyhow::{anyhow, Result};
use orca_whirlpools_core::{tick_index_to_sqrt_price, try_get_amount_delta_a, try_get_amount_delta_b};

/// Token amounts held in a position, in raw on-chain units (before decimal adjustment).
#[derive(Debug, Clone, PartialEq)]
pub struct TokenAmounts {
    pub amount_a: u64,
    pub amount_b: u64,
}

/// Compute token amounts for a position.
///
/// - liquidity: position's liquidity (u128 from account)
/// - sqrt_price: pool's current sqrt_price in Q64.64 (u128 from account)
/// - tick_lower / tick_upper: position range bounds
pub fn compute_token_amounts(
    liquidity: u128,
    sqrt_price: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> Result<TokenAmounts> {
    let sqrt_lower = tick_index_to_sqrt_price(tick_lower);
    let sqrt_upper = tick_index_to_sqrt_price(tick_upper);

    let (amount_a, amount_b) = if sqrt_price < sqrt_lower {
        // Price below range: all token A
        let a = try_get_amount_delta_a(sqrt_lower, sqrt_upper, liquidity, false)
            .map_err(|e| anyhow!("Failed to compute amount A: {:?}", e))?;
        (a, 0u64)
    } else if sqrt_price >= sqrt_upper {
        // Price above range: all token B
        let b = try_get_amount_delta_b(sqrt_lower, sqrt_upper, liquidity, false)
            .map_err(|e| anyhow!("Failed to compute amount B: {:?}", e))?;
        (0u64, b)
    } else {
        // Price in range: both tokens
        let a = try_get_amount_delta_a(sqrt_price, sqrt_upper, liquidity, false)
            .map_err(|e| anyhow!("Failed to compute amount A: {:?}", e))?;
        let b = try_get_amount_delta_b(sqrt_lower, sqrt_price, liquidity, false)
            .map_err(|e| anyhow!("Failed to compute amount B: {:?}", e))?;
        (a, b)
    };

    Ok(TokenAmounts { amount_a, amount_b })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqrt_price_at_tick(tick: i32) -> u128 {
        tick_index_to_sqrt_price(tick)
    }

    #[test]
    fn test_price_below_range_all_token_a_no_token_b() {
        // Price below range: all liquidity is token A
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(50),  // current below range [100, 200]
            100,
            200,
        ).unwrap();
        assert!(amounts.amount_a > 0, "token A should be > 0 below range");
        assert_eq!(amounts.amount_b, 0, "token B should be 0 below range");
    }

    #[test]
    fn test_price_above_range_all_token_b_no_token_a() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(300), // current above range [100, 200]
            100,
            200,
        ).unwrap();
        assert_eq!(amounts.amount_a, 0, "token A should be 0 above range");
        assert!(amounts.amount_b > 0, "token B should be > 0 above range");
    }

    #[test]
    fn test_price_in_range_has_both_tokens() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(150), // in range [100, 200]
            100,
            200,
        ).unwrap();
        assert!(amounts.amount_a > 0, "token A should be > 0 in range");
        assert!(amounts.amount_b > 0, "token B should be > 0 in range");
    }

    #[test]
    fn test_zero_liquidity_returns_zero_amounts() {
        let amounts = compute_token_amounts(
            0,
            sqrt_price_at_tick(150),
            100,
            200,
        ).unwrap();
        assert_eq!(amounts.amount_a, 0);
        assert_eq!(amounts.amount_b, 0);
    }
}