//! Exact Leviathan rejection sampling and speculative model pairing subsystem.
//!
//! Pairs fast CPU draft models with deep GPU primary models using distribution-preserving
//! Leviathan rejection sampling: min(1, p(x)/q(x)) with (p(x) - q(x))⁺ residual recovery.

use serde::{Deserialize, Serialize};

/// Paired speculative decoding models sharing identical tokenizer vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeculativePair {
    /// Fast CPU draft model identifier (e.g. "qwen2.5-0.5b").
    pub cpu_draft_model: String,
    /// Primary GPU target model identifier (e.g. "qwen2.5-7b").
    pub gpu_target_model: String,
    /// Shared tokenizer vocabulary family key (e.g. "qwen2").
    pub tokenizer_family: String,
}

impl SpeculativePair {
    /// Canonical Qwen 2.5 speculative pair: Qwen2.5-0.5B (CPU) -> Qwen2.5-7B (GPU).
    pub fn qwen_default() -> Self {
        Self {
            cpu_draft_model: "qwen2.5-0.5b".to_string(),
            gpu_target_model: "qwen2.5-7b".to_string(),
            tokenizer_family: "qwen2".to_string(),
        }
    }

    /// Drop-in reasoning pair: DeepSeek-R1-Distill-Qwen-1.5B (CPU) -> DeepSeek-R1-Distill-Qwen-14B (GPU).
    pub fn reasoning_pair() -> Self {
        Self {
            cpu_draft_model: "deepseek-r1-distill-qwen-1.5b".to_string(),
            gpu_target_model: "deepseek-r1-distill-qwen-14b".to_string(),
            tokenizer_family: "qwen2".to_string(),
        }
    }

    /// Identifies whether a model name belongs to drop-in reasoning distillation providers.
    pub fn is_reasoning_provider(model: &str) -> bool {
        let m = model.to_ascii_lowercase();
        m.contains("deepseek-r1-distill-qwen") || m.contains("r1-distill-qwen")
    }

    /// Validates that both candidate models share the same tokenizer vocabulary plane.
    pub fn shares_tokenizer(&self, other_model: &str) -> bool {
        let m = other_model.to_ascii_lowercase();
        if self.tokenizer_family == "qwen2" {
            m.contains("qwen") || m.contains("deepseek-r1-distill-qwen")
        } else {
            m.contains(&self.tokenizer_family)
        }
    }
}

/// Structured outcome of Leviathan speculative verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeviathanSampleResult {
    /// Sequence of accepted or corrected tokens emitted by the step.
    pub accepted_tokens: Vec<u32>,
    /// Number of draft tokens verified and accepted before rejection.
    pub accepted_count: usize,
    /// Total candidate tokens proposed by the draft model.
    pub proposed_count: usize,
    /// Optional bonus token if all proposed candidates were accepted.
    pub bonus_token: Option<u32>,
    /// Whether any draft token was rejected during verification.
    pub hit_rejection: bool,
}

/// Exact distribution-preserving Leviathan rejection sampling: min(1, p(x)/q(x)).
///
/// When a candidate token x is rejected, a correction token is sampled from
/// the positive residual distribution (p(x) - q(x))⁺. If all K candidates are accepted,
/// an optional bonus token sampled from the final target distribution is emitted.
pub fn sample_leviathan_speculation(
    draft_tokens: &[u32],
    draft_distributions: &[Vec<f32>],
    target_distributions: &[Vec<f32>],
    mut rand_source: impl FnMut() -> f32,
    bonus_candidate: Option<u32>,
) -> LeviathanSampleResult {
    let mut accepted_tokens = Vec::new();
    let mut accepted_count = 0;
    let mut hit_rejection = false;

    let k = draft_tokens.len().min(draft_distributions.len()).min(target_distributions.len());

    for i in 0..k {
        let tok = draft_tokens[i];
        let tok_idx = tok as usize;
        let p_dist = &target_distributions[i];
        let q_dist = &draft_distributions[i];

        let p_x = p_dist.get(tok_idx).copied().unwrap_or(0.0);
        let q_x = q_dist.get(tok_idx).copied().unwrap_or(0.0);

        let accept = if q_x <= 0.0 || q_x.is_nan() {
            p_x > 0.0
        } else if p_x >= q_x {
            true
        } else {
            let alpha = p_x / q_x;
            let u = rand_source().clamp(0.0, 1.0);
            u < alpha
        };

        if accept {
            accepted_tokens.push(tok);
            accepted_count += 1;
        } else {
            hit_rejection = true;
            let vocab_size = p_dist.len().max(q_dist.len());
            let mut residual = Vec::with_capacity(vocab_size);
            let mut residual_sum = 0.0f32;

            for idx in 0..vocab_size {
                let p_v = p_dist.get(idx).copied().unwrap_or(0.0);
                let q_v = q_dist.get(idx).copied().unwrap_or(0.0);
                let diff = (p_v - q_v).max(0.0);
                residual.push(diff);
                residual_sum += diff;
            }

            let correction = if residual_sum > 1e-8 && !residual_sum.is_nan() {
                sample_categorical(&residual, residual_sum, &mut rand_source)
            } else {
                let p_sum: f32 = p_dist.iter().sum();
                sample_categorical(p_dist, p_sum, &mut rand_source)
            };

            accepted_tokens.push(correction);
            break;
        }
    }

    let bonus_token = if !hit_rejection && accepted_count == draft_tokens.len() {
        if let Some(bonus) = bonus_candidate {
            accepted_tokens.push(bonus);
            Some(bonus)
        } else {
            None
        }
    } else {
        None
    };

    LeviathanSampleResult {
        accepted_tokens,
        accepted_count,
        proposed_count: draft_tokens.len(),
        bonus_token,
        hit_rejection,
    }
}

fn sample_categorical(probs: &[f32], total_mass: f32, rand_source: &mut impl FnMut() -> f32) -> u32 {
    let r = rand_source().clamp(0.0, 1.0) * total_mass.max(1e-8);
    let mut cumsum = 0.0f32;
    for (idx, &p) in probs.iter().enumerate() {
        cumsum += p;
        if r <= cumsum {
            return idx as u32;
        }
    }
    (probs.len().saturating_sub(1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qwen_speculative_pair_defaults() {
        let pair = SpeculativePair::qwen_default();
        assert_eq!(pair.cpu_draft_model, "qwen2.5-0.5b");
        assert_eq!(pair.gpu_target_model, "qwen2.5-7b");
        assert!(pair.shares_tokenizer("deepseek-r1-distill-qwen-14b"));
        assert!(pair.shares_tokenizer("qwen2.5-coder-7b"));
    }

    #[test]
    fn test_reasoning_speculative_pair() {
        let pair = SpeculativePair::reasoning_pair();
        assert_eq!(pair.cpu_draft_model, "deepseek-r1-distill-qwen-1.5b");
        assert_eq!(pair.gpu_target_model, "deepseek-r1-distill-qwen-14b");
        assert!(SpeculativePair::is_reasoning_provider("deepseek-r1-distill-qwen-1.5b"));
        assert!(SpeculativePair::is_reasoning_provider("deepseek-r1-distill-qwen-14b"));
        assert!(!SpeculativePair::is_reasoning_provider("qwen2.5-7b"));
    }

    #[test]
    fn test_leviathan_all_accepted_with_bonus() {
        let draft = vec![1, 2];
        let q_dist = vec![vec![0.0, 0.5, 0.5], vec![0.0, 0.5, 0.5]];
        let p_dist = vec![vec![0.0, 0.8, 0.2], vec![0.0, 0.2, 0.8]];

        // With p >= q, alpha = 1.0, guaranteed accept
        let res = sample_leviathan_speculation(&draft, &q_dist, &p_dist, || 0.1, Some(99));
        assert_eq!(res.accepted_count, 2);
        assert_eq!(res.bonus_token, Some(99));
        assert_eq!(res.accepted_tokens, vec![1, 2, 99]);
        assert!(!res.hit_rejection);
    }

    #[test]
    fn test_leviathan_rejection_and_residual_correction() {
        let draft = vec![1];
        let q_dist = vec![vec![0.0, 0.9, 0.1]];
        let p_dist = vec![vec![0.0, 0.1, 0.9]];

        // alpha = 0.1 / 0.9 = 0.111. With rand = 0.5 > alpha, candidate rejected!
        let res = sample_leviathan_speculation(&draft, &q_dist, &p_dist, || 0.5, None);
        assert_eq!(res.accepted_count, 0);
        assert!(res.hit_rejection);
        assert_eq!(res.accepted_tokens.len(), 1);
        // Residual is strictly at token 2 (p=0.9, q=0.1 -> diff=0.8)
        assert_eq!(res.accepted_tokens[0], 2);
    }
}
