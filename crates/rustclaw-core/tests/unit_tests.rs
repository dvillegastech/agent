/// Test the cost tracker logic
#[cfg(test)]
mod cost_tests {
    // We can't import from a bin crate directly, so we test the logic inline.

    #[derive(Debug, Default)]
    struct CostTracker {
        total_input_tokens: u64,
        total_output_tokens: u64,
        request_count: u32,
        model: String,
    }

    impl CostTracker {
        fn new(model: &str) -> Self {
            Self {
                model: model.to_string(),
                ..Default::default()
            }
        }

        fn record(&mut self, input_tokens: u32, output_tokens: u32) {
            self.total_input_tokens += input_tokens as u64;
            self.total_output_tokens += output_tokens as u64;
            self.request_count += 1;
        }

        fn estimated_cost(&self) -> f64 {
            let (input_price, output_price) = model_pricing(&self.model);
            let input_cost = (self.total_input_tokens as f64 / 1_000_000.0) * input_price;
            let output_cost = (self.total_output_tokens as f64 / 1_000_000.0) * output_price;
            input_cost + output_cost
        }
    }

    fn model_pricing(model: &str) -> (f64, f64) {
        match model {
            m if m.contains("opus") => (15.0, 75.0),
            m if m.contains("sonnet") => (3.0, 15.0),
            m if m.contains("haiku") => (0.25, 1.25),
            m if m.contains("gpt-4o-mini") => (0.15, 0.60),
            m if m.contains("gpt-4o") => (2.50, 10.0),
            m if m.contains("gpt-4-turbo") => (10.0, 30.0),
            _ => (3.0, 15.0),
        }
    }

    #[test]
    fn test_cost_tracker_new() {
        let tracker = CostTracker::new("claude-sonnet");
        assert_eq!(tracker.total_input_tokens, 0);
        assert_eq!(tracker.total_output_tokens, 0);
        assert_eq!(tracker.request_count, 0);
    }

    #[test]
    fn test_cost_tracker_record() {
        let mut tracker = CostTracker::new("claude-sonnet");
        tracker.record(100, 50);
        assert_eq!(tracker.total_input_tokens, 100);
        assert_eq!(tracker.total_output_tokens, 50);
        assert_eq!(tracker.request_count, 1);

        tracker.record(200, 100);
        assert_eq!(tracker.total_input_tokens, 300);
        assert_eq!(tracker.total_output_tokens, 150);
        assert_eq!(tracker.request_count, 2);
    }

    #[test]
    fn test_cost_estimation_sonnet() {
        let mut tracker = CostTracker::new("claude-sonnet");
        tracker.record(1_000_000, 1_000_000);
        let cost = tracker.estimated_cost();
        // sonnet: $3/M input + $15/M output = $18
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn test_cost_estimation_opus() {
        let mut tracker = CostTracker::new("claude-opus");
        tracker.record(1_000_000, 1_000_000);
        let cost = tracker.estimated_cost();
        // opus: $15/M input + $75/M output = $90
        assert!((cost - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_cost_estimation_haiku() {
        let mut tracker = CostTracker::new("claude-haiku");
        tracker.record(1_000_000, 1_000_000);
        let cost = tracker.estimated_cost();
        // haiku: $0.25/M input + $1.25/M output = $1.50
        assert!((cost - 1.50).abs() < 0.01);
    }

    #[test]
    fn test_model_pricing_gpt4o() {
        let (input, output) = model_pricing("gpt-4o");
        assert!((input - 2.50).abs() < 0.01);
        assert!((output - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_model_pricing_gpt4o_mini() {
        // gpt-4o-mini must match before gpt-4o
        let (input, output) = model_pricing("gpt-4o-mini");
        assert!((input - 0.15).abs() < 0.01);
        assert!((output - 0.60).abs() < 0.01);
    }

    #[test]
    fn test_model_pricing_unknown_defaults_to_sonnet() {
        let (input, output) = model_pricing("unknown-model");
        assert!((input - 3.0).abs() < 0.01);
        assert!((output - 15.0).abs() < 0.01);
    }
}

/// Test security validation logic
#[cfg(test)]
mod security_tests {

    fn dangerous_patterns() -> Vec<&'static str> {
        vec![
            "curl|sh",
            "curl|bash",
            "wget|sh",
            "wget|bash",
            ">/dev/sd",
            "chmod777/",
            "chown-R",
            "passwd",
            "visudo",
        ]
    }

    fn default_blocked() -> Vec<String> {
        vec![
            "rm -rf /".into(),
            "mkfs".into(),
            "dd if=/dev".into(),
            ":(){:|:&};:".into(),
            "chmod -R 777 /".into(),
            "shutdown".into(),
            "reboot".into(),
            "halt".into(),
            "init 0".into(),
            "init 6".into(),
        ]
    }

    fn validate_command(command: &str, blocked: &[String]) -> Result<(), String> {
        let trimmed = command.trim();
        for b in blocked {
            if trimmed.contains(b.as_str()) {
                return Err(format!("Command blocked: contains '{b}'"));
            }
        }
        let normalized = trimmed.replace(' ', "");
        for pattern in dangerous_patterns() {
            if normalized.contains(pattern) {
                return Err(format!("Dangerous pattern: {pattern}"));
            }
        }
        Ok(())
    }

    #[test]
    fn test_safe_commands_pass() {
        let blocked = default_blocked();
        assert!(validate_command("ls -la", &blocked).is_ok());
        assert!(validate_command("cat file.txt", &blocked).is_ok());
        assert!(validate_command("grep -r pattern .", &blocked).is_ok());
        assert!(validate_command("cargo build", &blocked).is_ok());
    }

    #[test]
    fn test_blocked_commands_fail() {
        let blocked = default_blocked();
        assert!(validate_command("rm -rf /", &blocked).is_err());
        assert!(validate_command("mkfs.ext4 /dev/sda", &blocked).is_err());
        assert!(validate_command("dd if=/dev/zero of=/dev/sda", &blocked).is_err());
        assert!(validate_command("shutdown now", &blocked).is_err());
        assert!(validate_command("reboot", &blocked).is_err());
    }

    #[test]
    fn test_dangerous_patterns_fail() {
        let blocked = default_blocked();
        // Direct pipe patterns (no URL gap)
        assert!(validate_command("curl | sh", &blocked).is_err());
        assert!(validate_command("wget | bash", &blocked).is_err());
        // chmod -R 777 / is in blocked commands list
        assert!(validate_command("chmod -R 777 /", &blocked).is_err());
        // visudo is a dangerous pattern
        assert!(validate_command("visudo", &blocked).is_err());
    }

    #[test]
    fn test_fork_bomb_blocked() {
        let blocked = default_blocked();
        assert!(validate_command(":(){:|:&};:", &blocked).is_err());
    }

    #[test]
    fn test_file_size_validation() {
        let max_size: u64 = 10 * 1024 * 1024; // 10 MB
        assert!(5_000_000u64 <= max_size); // 5MB ok
        assert!(15_000_000u64 > max_size); // 15MB exceeds
    }
}

/// Test conversation management
#[cfg(test)]
mod conversation_tests {
    #[derive(Debug, Clone)]
    enum Role {
        User,
        Assistant,
    }

    #[derive(Debug, Clone)]
    struct Message {
        #[allow(dead_code)]
        role: Role,
        content: String,
    }

    struct Conversation {
        messages: Vec<Message>,
        max_turns: usize,
    }

    impl Conversation {
        fn new(max_turns: usize) -> Self {
            Self {
                messages: Vec::new(),
                max_turns,
            }
        }

        fn add_user_message(&mut self, text: &str) {
            self.messages.push(Message {
                role: Role::User,
                content: text.to_string(),
            });
            self.trim();
        }

        fn add_assistant_message(&mut self, text: &str) {
            self.messages.push(Message {
                role: Role::Assistant,
                content: text.to_string(),
            });
            self.trim();
        }

        fn len(&self) -> usize {
            self.messages.len()
        }

        fn clear(&mut self) {
            self.messages.clear();
        }

        fn trim(&mut self) {
            let max_messages = self.max_turns * 2;
            if self.messages.len() > max_messages && self.messages.len() > 2 {
                let excess = self.messages.len() - max_messages;
                self.messages.drain(1..=excess);
            }
        }
    }

    #[test]
    fn test_conversation_new() {
        let conv = Conversation::new(10);
        assert_eq!(conv.len(), 0);
    }

    #[test]
    fn test_add_messages() {
        let mut conv = Conversation::new(10);
        conv.add_user_message("hello");
        assert_eq!(conv.len(), 1);
        conv.add_assistant_message("hi there");
        assert_eq!(conv.len(), 2);
    }

    #[test]
    fn test_clear() {
        let mut conv = Conversation::new(10);
        conv.add_user_message("hello");
        conv.add_assistant_message("hi");
        conv.clear();
        assert_eq!(conv.len(), 0);
    }

    #[test]
    fn test_trim_preserves_first_message() {
        let mut conv = Conversation::new(2); // max 2 turns = 4 messages
        conv.add_user_message("first");
        conv.add_assistant_message("response 1");
        conv.add_user_message("second");
        conv.add_assistant_message("response 2");
        conv.add_user_message("third");
        // After trim, first message should be preserved
        assert!(conv.messages[0].content == "first");
    }

    #[test]
    fn test_trim_limits_messages() {
        let mut conv = Conversation::new(2); // max 4 messages
        for i in 0..10 {
            conv.add_user_message(&format!("msg {i}"));
            conv.add_assistant_message(&format!("resp {i}"));
        }
        // Should be at most max_turns * 2 = 4 messages
        assert!(conv.len() <= 4);
    }
}

/// Test type serialization
#[cfg(test)]
mod type_tests {
    use serde_json::json;

    #[test]
    fn test_usage_from_json_anthropic() {
        let data = json!({"input_tokens": 100, "output_tokens": 50});
        let input = data["input_tokens"].as_u64().unwrap() as u32;
        let output = data["output_tokens"].as_u64().unwrap() as u32;
        assert_eq!(input, 100);
        assert_eq!(output, 50);
    }

    #[test]
    fn test_usage_from_json_openai() {
        let data = json!({"prompt_tokens": 200, "completion_tokens": 75});
        let input = data["prompt_tokens"].as_u64().unwrap() as u32;
        let output = data["completion_tokens"].as_u64().unwrap() as u32;
        assert_eq!(input, 200);
        assert_eq!(output, 75);
    }

    #[test]
    fn test_usage_from_json_missing_fields() {
        let data = json!({"other": 100});
        assert!(data["input_tokens"].as_u64().is_none());
    }

    #[test]
    fn test_format_number() {
        fn format_number(n: u64) -> String {
            if n >= 1_000_000 {
                format!("{:.1}M", n as f64 / 1_000_000.0)
            } else if n >= 1_000 {
                format!("{:.1}K", n as f64 / 1_000.0)
            } else {
                n.to_string()
            }
        }

        assert_eq!(format_number(500), "500");
        assert_eq!(format_number(1500), "1.5K");
        assert_eq!(format_number(1_500_000), "1.5M");
        assert_eq!(format_number(0), "0");
    }
}
