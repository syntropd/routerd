//! Elastic model family downgrading stepping down 7B -> 1.5B -> 0.5B under VRAM pressure without context loss.

use crate::models::ChatCompletionRequest;

/// VRAM memory pressure tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VramPressureLevel {
    /// Normal operation (<75% VRAM, low PSI): retain primary target model.
    Normal,
    /// Elevated pressure (75%-90% VRAM, or PSI some > 25%): step down 7B -> 1.5B.
    Elevated,
    /// Critical pressure (>90% VRAM, or PSI full > 5%): step down to 0.5B.
    Critical,
}

/// Outcome of elastic model family downgrade evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DowngradeDecision {
    /// Model kept unchanged.
    Unchanged { model: String },
    /// Elastic downgrade applied without context loss.
    Downgraded {
        from: String,
        to: String,
        level: VramPressureLevel,
        prompt_tokens: usize,
    },
    /// Downgrade rejected because prompt tokens exceed target model context window.
    ContextExceeded {
        attempted: String,
        prompt_tokens: usize,
        context_window: usize,
    },
}

/// Ladder of family models with context boundaries.
#[derive(Debug, Clone)]
pub struct FamilyLadder {
    pub model_7b: String,
    pub model_1_5b: String,
    pub model_0_5b: String,
    pub context_7b: usize,
    pub context_1_5b: usize,
    pub context_0_5b: usize,
}

impl Default for FamilyLadder {
    fn default() -> Self {
        Self {
            model_7b: "qwen2.5-7b".into(),
            model_1_5b: "qwen2.5-1.5b".into(),
            model_0_5b: "qwen2.5-0.5b".into(),
            context_7b: 32768,
            context_1_5b: 32768,
            context_0_5b: 32768,
        }
    }
}

/// Elastic downgrader managing family step-down under hardware pressure.
#[derive(Debug, Clone)]
pub struct ElasticFamilyDowngrader {
    ladder: FamilyLadder,
    elevated_vram_ratio: f64,
    critical_vram_ratio: f64,
    elevated_psi_some: f64,
    critical_psi_full: f64,
}

impl Default for ElasticFamilyDowngrader {
    fn default() -> Self {
        Self {
            ladder: FamilyLadder::default(),
            elevated_vram_ratio: 0.75,
            critical_vram_ratio: 0.90,
            elevated_psi_some: 25.0,
            critical_psi_full: 5.0,
        }
    }
}

impl ElasticFamilyDowngrader {
    /// Create an ElasticFamilyDowngrader with custom family model ladder.
    pub fn new(ladder: FamilyLadder) -> Self {
        Self {
            ladder,
            ..Default::default()
        }
    }

    /// Assess VRAM pressure tier from telemetry metrics.
    pub fn assess_pressure(
        &self,
        vram_used: u64,
        vram_total: u64,
        psi_some: f64,
        psi_full: f64,
    ) -> VramPressureLevel {
        let ratio = if vram_total > 0 {
            vram_used as f64 / vram_total as f64
        } else {
            0.0
        };

        if ratio >= self.critical_vram_ratio || psi_full >= self.critical_psi_full {
            VramPressureLevel::Critical
        } else if ratio >= self.elevated_vram_ratio || psi_some >= self.elevated_psi_some {
            VramPressureLevel::Elevated
        } else {
            VramPressureLevel::Normal
        }
    }

    /// Evaluate and apply elastic step-down (7B -> 1.5B -> 0.5B) preserving context.
    pub fn evaluate(
        &self,
        req: &mut ChatCompletionRequest,
        pressure: VramPressureLevel,
    ) -> DowngradeDecision {
        let current_model = req.model.to_ascii_lowercase();
        let prompt_tokens = req.estimate_prompt_tokens();

        let is_7b = current_model == self.ladder.model_7b.to_ascii_lowercase() || current_model.contains("7b");
        let is_1_5b = current_model == self.ladder.model_1_5b.to_ascii_lowercase() || current_model.contains("1.5b");

        let target_spec = match pressure {
            VramPressureLevel::Normal => None,
            VramPressureLevel::Elevated => {
                if is_7b {
                    Some((&self.ladder.model_1_5b, self.ladder.context_1_5b))
                } else {
                    None
                }
            }
            VramPressureLevel::Critical => {
                if is_7b || is_1_5b {
                    Some((&self.ladder.model_0_5b, self.ladder.context_0_5b))
                } else {
                    None
                }
            }
        };

        if let Some((target_model, context_limit)) = target_spec {
            if prompt_tokens > context_limit {
                return DowngradeDecision::ContextExceeded {
                    attempted: target_model.clone(),
                    prompt_tokens,
                    context_window: context_limit,
                };
            }
            let from = std::mem::replace(&mut req.model, target_model.clone());
            DowngradeDecision::Downgraded {
                from,
                to: target_model.clone(),
                level: pressure,
                prompt_tokens,
            }
        } else {
            DowngradeDecision::Unchanged {
                model: req.model.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatMessage;

    #[test]
    fn test_normal_pressure_retains_7b() {
        let downgrader = ElasticFamilyDowngrader::default();
        let mut req = ChatCompletionRequest {
            model: "qwen2.5-7b".into(),
            messages: vec![ChatMessage::new("user", "test prompt")],
            ..Default::default()
        };
        let decision = downgrader.evaluate(&mut req, VramPressureLevel::Normal);
        assert_eq!(decision, DowngradeDecision::Unchanged { model: "qwen2.5-7b".into() });
        assert_eq!(req.model, "qwen2.5-7b");
    }

    #[test]
    fn test_elevated_pressure_steps_down_to_1_5b() {
        let downgrader = ElasticFamilyDowngrader::default();
        let mut req = ChatCompletionRequest {
            model: "qwen2.5-7b".into(),
            messages: vec![ChatMessage::new("user", "test prompt")],
            ..Default::default()
        };
        let decision = downgrader.evaluate(&mut req, VramPressureLevel::Elevated);
        assert!(matches!(decision, DowngradeDecision::Downgraded { to, level: VramPressureLevel::Elevated, .. } if to == "qwen2.5-1.5b"));
        assert_eq!(req.model, "qwen2.5-1.5b");
    }

    #[test]
    fn test_critical_pressure_steps_down_to_0_5b() {
        let downgrader = ElasticFamilyDowngrader::default();
        let mut req = ChatCompletionRequest {
            model: "qwen2.5-7b".into(),
            messages: vec![ChatMessage::new("user", "test prompt")],
            ..Default::default()
        };
        let decision = downgrader.evaluate(&mut req, VramPressureLevel::Critical);
        assert!(matches!(decision, DowngradeDecision::Downgraded { to, level: VramPressureLevel::Critical, .. } if to == "qwen2.5-0.5b"));
        assert_eq!(req.model, "qwen2.5-0.5b");
    }

    #[test]
    fn test_custom_ladder_downgrade() {
        let ladder = FamilyLadder {
            model_7b: "custom-large".into(),
            model_1_5b: "custom-mid".into(),
            model_0_5b: "custom-small".into(),
            context_7b: 16384,
            context_1_5b: 16384,
            context_0_5b: 16384,
        };
        let downgrader = ElasticFamilyDowngrader::new(ladder);
        let mut req = ChatCompletionRequest {
            model: "custom-large".into(),
            messages: vec![ChatMessage::new("user", "test prompt")],
            ..Default::default()
        };
        let decision = downgrader.evaluate(&mut req, VramPressureLevel::Elevated);
        assert!(matches!(decision, DowngradeDecision::Downgraded { to, .. } if to == "custom-mid"));
        assert_eq!(req.model, "custom-mid");
    }
}
