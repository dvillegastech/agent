use colored::Colorize;

/// Tracks token usage and estimated costs across a session.
#[derive(Debug, Default)]
pub struct CostTracker {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub request_count: u32,
    model: String,
}

impl CostTracker {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            ..Default::default()
        }
    }

    /// Record token usage from a response.
    pub fn record(&mut self, input_tokens: u32, output_tokens: u32) {
        self.total_input_tokens += input_tokens as u64;
        self.total_output_tokens += output_tokens as u64;
        self.request_count += 1;
    }

    /// Estimated cost in USD based on model pricing.
    pub fn estimated_cost(&self) -> f64 {
        let (input_price, output_price) = model_pricing(&self.model);
        let input_cost = (self.total_input_tokens as f64 / 1_000_000.0) * input_price;
        let output_cost = (self.total_output_tokens as f64 / 1_000_000.0) * output_price;
        input_cost + output_cost
    }

    /// Format a summary for display.
    pub fn summary(&self) -> String {
        let cost = self.estimated_cost();
        format!(
            "Session: {} requests | {} input + {} output tokens | ~${:.4}",
            self.request_count,
            format_number(self.total_input_tokens),
            format_number(self.total_output_tokens),
            cost,
        )
    }

    /// Print a brief token update to stderr.
    pub fn print_update(&self, input: u32, output: u32) {
        let cost = self.estimated_cost();
        eprintln!(
            "{}",
            format!(
                "  [tokens] +{}in/+{}out | total: {}in/{}out | ~${:.4}",
                input,
                output,
                format_number(self.total_input_tokens),
                format_number(self.total_output_tokens),
                cost,
            )
            .dimmed()
        );
    }
}

/// Returns (input_price_per_mtok, output_price_per_mtok) in USD.
fn model_pricing(model: &str) -> (f64, f64) {
    match model {
        // Anthropic
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("sonnet") => (3.0, 15.0),
        m if m.contains("haiku") => (0.25, 1.25),
        // OpenAI
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("gpt-4-turbo") => (10.0, 30.0),
        // Ollama / local models (free)
        m if m.contains("llama") || m.contains("qwen") || m.contains("mistral")
            || m.contains("codestral") || m.contains("deepseek") || m.contains("phi")
            || m.contains("gemma") => (0.0, 0.0),
        // Default fallback
        _ => (3.0, 15.0),
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
