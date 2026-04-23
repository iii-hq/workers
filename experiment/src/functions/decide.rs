use serde_json::Value;

pub const DIRECTION_MINIMIZE: &str = "minimize";
pub const DIRECTION_MAXIMIZE: &str = "maximize";

pub struct Decision {
    pub kept: bool,
    pub reason: String,
    pub improvement_pct: Option<f64>,
}

pub fn evaluate(definition: &Value, score: f64, current_best: f64) -> Decision {
    let direction = definition
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or(DIRECTION_MINIMIZE);

    let is_better = match direction {
        DIRECTION_MINIMIZE => score < current_best,
        DIRECTION_MAXIMIZE => score > current_best,
        _ => false,
    };

    let improvement_pct = if current_best.abs() > f64::EPSILON {
        Some(((score - current_best) / current_best.abs()) * 100.0)
    } else {
        Some(0.0)
    };

    if is_better {
        Decision {
            kept: true,
            reason: format!(
                "score {} is better than current best {} ({})",
                score, current_best, direction
            ),
            improvement_pct,
        }
    } else {
        Decision {
            kept: false,
            reason: format!(
                "score {} is not better than current best {} ({})",
                score, current_best, direction
            ),
            improvement_pct,
        }
    }
}

pub async fn handle(payload: Value) -> Result<Value, iii_sdk::IIIError> {
    let experiment_id = payload
        .get("experiment_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let score = payload
        .get("score")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| iii_sdk::IIIError::Handler("missing required field: score".to_string()))?;

    let iteration = payload
        .get("iteration")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let current_best = payload
        .get("current_best")
        .and_then(|v| v.as_f64())
        .unwrap_or(score);

    let direction = payload
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("minimize");

    let definition = serde_json::json!({ "direction": direction });
    let decision = evaluate(&definition, score, current_best);

    Ok(serde_json::json!({
        "experiment_id": experiment_id,
        "iteration": iteration,
        "kept": decision.kept,
        "reason": decision.reason,
        "improvement_pct": decision.improvement_pct,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_minimize_better() {
        let def = json!({"direction": "minimize"});
        let d = evaluate(&def, 50.0, 100.0);
        assert!(d.kept);
        assert!(d.reason.contains("better"));
    }

    #[test]
    fn test_minimize_worse() {
        let def = json!({"direction": "minimize"});
        let d = evaluate(&def, 150.0, 100.0);
        assert!(!d.kept);
    }

    #[test]
    fn test_maximize_better() {
        let def = json!({"direction": "maximize"});
        let d = evaluate(&def, 150.0, 100.0);
        assert!(d.kept);
    }

    #[test]
    fn test_maximize_worse() {
        let def = json!({"direction": "maximize"});
        let d = evaluate(&def, 50.0, 100.0);
        assert!(!d.kept);
    }

    #[test]
    fn test_equal_scores() {
        let def = json!({"direction": "minimize"});
        let d = evaluate(&def, 100.0, 100.0);
        assert!(!d.kept);
    }

    #[test]
    fn test_improvement_pct() {
        let def = json!({"direction": "minimize"});
        let d = evaluate(&def, 80.0, 100.0);
        assert!(d.kept);
        let pct = d.improvement_pct.unwrap();
        assert!((pct - (-20.0)).abs() < 0.01);
    }

    #[test]
    fn test_default_direction_minimize() {
        let def = json!({});
        let d = evaluate(&def, 50.0, 100.0);
        assert!(d.kept);
    }
}
