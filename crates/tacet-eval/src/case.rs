//! Eval cases — Tacet's behavioural contract, WRITTEN DOWN.

use serde::Serialize;

/// A single evaluation case.
#[derive(Debug, Clone, Serialize)]
pub struct EvalCase {
    pub name: String,
    pub input: String,
    pub expected_tool: Option<String>,
    pub expected_evidence: Vec<String>,
    pub forbidden: Vec<String>,
    #[serde(skip)]
    pub script: Vec<String>,
    #[serde(skip)]
    pub unconstrained: bool,
}

impl EvalCase {
    pub fn new(name: &str, input: &str) -> Self {
        Self {
            name: name.into(),
            input: input.into(),
            expected_tool: None,
            expected_evidence: Vec::new(),
            forbidden: Vec::new(),
            script: Vec::new(),
            unconstrained: false,
        }
    }

    pub fn unconstrained(mut self) -> Self {
        self.unconstrained = true;
        self
    }

    pub fn tool(mut self, name: &str) -> Self {
        self.expected_tool = Some(name.into());
        self
    }

    pub fn evidence(mut self, parts: &[&str]) -> Self {
        self.expected_evidence = parts.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn forbidden(mut self, parts: &[&str]) -> Self {
        self.forbidden = parts.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn script(mut self, steps: &[&str]) -> Self {
        self.script = steps.iter().map(|s| s.to_string()).collect();
        self
    }
}

pub const TABLE_FILE: &str = "report.md";
pub const LONG_FILE: &str = "long.md";
pub const BUDGET_FILE: &str = "budget-2026.md";
pub const FIXED_EPOCH: i64 = 1_784_505_600;

pub fn all() -> Vec<EvalCase> {
    let mut v = Vec::new();
    v.extend(chat());
    v.extend(calc());
    v.extend(time());
    v.extend(document());
    v.extend(channel());
    v.extend(gate());
    v
}

fn chat() -> Vec<EvalCase> {
    vec![
        EvalCase::new("chat-greeting", "Hello")
            .script(&["Hello! How can I help?"])
            .evidence(&["Hello"]),
        EvalCase::new("chat-thanks", "Thank you very much")
            .script(&["You're welcome."])
            .evidence(&["welcome"]),
        EvalCase::new("chat-on-device", "Are you sending my data to the cloud?")
            .script(&["No, everything stays on your device."])
            .evidence(&["on your device"])
            .forbidden(&["to our servers"]),
        EvalCase::new("chat-identity", "Who created you?")
            .script(&["I am Tacet, an on-device AI assistant."])
            .evidence(&["Tacet"])
            .forbidden(&["ChatGPT", "OpenAI", "Anthropic"]),
        EvalCase::new("chat-capabilities", "What can you do?")
            .script(&["I can help with document editing, file search, calculations, and local notes."])
            .evidence(&["document", "calculations"]),
        EvalCase::new("chat-farewell", "Goodbye, see you later!")
            .script(&["Goodbye! Have a great day."])
            .evidence(&["Goodbye"]),
        EvalCase::new("chat-opinion", "Which is better, morning or evening workout?")
            .script(&["Both have benefits depending on your schedule and energy levels."])
            .evidence(&["benefits"]),
        EvalCase::new("chat-continuation", "Tell me more about that")
            .script(&["Sure, here are additional details."])
            .evidence(&["details"]),
        EvalCase::new("chat-shorter", "Can you explain it more briefly?")
            .script(&["In short: focus on consistency."])
            .evidence(&["In short"]),
        EvalCase::new("chat-general-knowledge", "What is the capital of France?")
            .script(&["The capital of France is Paris."])
            .evidence(&["Paris"])
            .forbidden(&["web_search"]),
    ]
}

fn calc() -> Vec<EvalCase> {
    vec![
        EvalCase::new("calc-multiply", "What is 125 times 8?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"125*8"})"#, "125 x 8 = 1000."])
            .evidence(&["1000"]),
        EvalCase::new("calc-percent", "How much is 250 lira with a 20 percent discount?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"250-250*20%"})"#, "200 lira."])
            .evidence(&["200"]),
        EvalCase::new("calc-add", "Could you add 347 and 268?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"347+268"})"#, "347 + 268 = 615."])
            .evidence(&["615"]),
        EvalCase::new("calc-divide", "What is 144 divided by 12?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"144/12"})"#, "144 / 12 = 12."])
            .evidence(&["12"]),
        EvalCase::new("calc-float", "What is 15.5 times 4.2?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"15.5*4.2"})"#, "15.5 x 4.2 = 65.1."])
            .evidence(&["65.1"]),
        EvalCase::new("calc-complex", "Calculate (45 + 55) * 12 / 4")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"(45+55)*12/4"})"#, "Result is 300."])
            .evidence(&["300"]),
        EvalCase::new("calc-invalid", "What is sin(45)?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"sin(45)"})"#, "I could not compute that."])
            .evidence(&["tool_failed"]),
        EvalCase::new("calc-syntax-error", "What is 12 ++ * 5?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"12++*5"})"#, "Syntax error in expression."])
            .evidence(&["tool_failed"]),
        EvalCase::new("calc-zero-division", "What is 100 divided by 0?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"100/0"})"#, "Division by zero is undefined."])
            .evidence(&["tool_failed"]),
        EvalCase::new("calc-large-number", "What is 999999 times 999999?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"999999*999999"})"#, "999998000001."])
            .evidence(&["999998000001"]),
    ]
}

fn time() -> Vec<EvalCase> {
    vec![
        EvalCase::new("time-date", "What is today's date?")
            .tool("time")
            .script(&[r#"time({"kind":"date"})"#, "Today is 20 July 2026."])
            .evidence(&["date=2026-07-20"]),
        EvalCase::new("time-weekday", "What day of the week is it today?")
            .tool("time")
            .script(&[r#"time({"kind":"weekday"})"#, "Monday."])
            .evidence(&["weekday=Monday"]),
        EvalCase::new("time-clock", "What time is it right now?")
            .tool("time")
            .script(&[r#"time({"kind":"clock"})"#, "It is 14:30."])
            .evidence(&["time=00:00"]),
        EvalCase::new("time-diff", "How many days until 2 December 2026?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-12-02"})"#,
                "135 days.",
            ])
            .evidence(&["days=135", "to=2026-12-02"]),
        EvalCase::new("time-year-end", "How many days until the end of the year?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-12-31"})"#,
                "164 days left.",
            ])
            .evidence(&["to=2026-12-31"]),
        EvalCase::new("time-unresolvable", "How many days until whatsit day?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"whatsit day"})"#,
                "I could not understand the date, could you clarify?",
            ])
            .evidence(&["unparsable_date"])
            .forbidden(&["days=0"]),
        EvalCase::new("time-past-date", "How many days since 1 January 2026?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-01-01"})"#,
                "200 days ago.",
            ])
            .evidence(&["2026-01-01"]),
        EvalCase::new("time-timezone", "What is the UTC time?")
            .tool("time")
            .script(&[
                r#"time({"kind":"clock"})"#,
                "The current UTC time is 12:00.",
            ])
            .evidence(&["time=00:00"]),
    ]
}

fn document() -> Vec<EvalCase> {
    vec![
        EvalCase::new("read-document-table", "What is in the file report.md?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                "The file has a weekly meal table.",
            ])
            .evidence(&["| Day | Meal |", "| --- |", "| Monday | Lentils |"]),
        EvalCase::new("create-document-excel", "Make an excel file for the weekly meal list")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"excel","file_name":"meals","content":"| Day | Meal |\n| --- | --- |\n| Monday | Lentils |"})"#,
                "I created the excel file.",
            ])
            .evidence(&["file_created (excel)", "meals.xlsx"]),
        EvalCase::new("create-document-markdown", "Create a short note file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"note","content":"Hello"})"#,
                "The note file is ready.",
            ])
            .evidence(&["file_created (markdown)", "note.md"]),
        EvalCase::new("read-document-missing", "Summarize the file missing.md")
            .tool("read_document")
            .script(&[r#"read_document({"path":"missing.md"})"#, "I could not find the file."])
            .evidence(&["tool_failed"]),
        EvalCase::new("document-schema-violation", "Create a file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"excel"})"#,
                "Could you tell me the file name?",
            ])
            .evidence(&["tool_failed"])
            .forbidden(&["file_created"])
            .unconstrained(),
    ]
}

fn channel() -> Vec<EvalCase> {
    vec![
        EvalCase::new("channel-source-ref", "What is in the file long.md?")
            .tool("read_document")
            .script(&[r#"read_document({"path":"long.md"})"#, "There is a long list."])
            .evidence(&["source_ref=document#1"]),
        EvalCase::new("channel-chain", "Dump the contents of long.md into a markdown file")
            .tool("create_document")
            .script(&[
                r#"read_document({"path":"long.md"})"#,
                r#"create_document({"format":"markdown","file_name":"dump","source_ref":"document#1"})"#,
                "I created the file.",
            ])
            .evidence(&["source_ref=document#1", "file_created (markdown)", "dump.md"]),
        EvalCase::new("channel-unknown-ref", "Dump the data in the store into a file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"ghost","source_ref":"document#99"})"#,
                "I could not find the source data.",
            ])
            .evidence(&["unknown_data_ref"])
            .forbidden(&["file_created"]),
    ]
}

fn gate() -> Vec<EvalCase> {
    vec![
        EvalCase::new("gate-clean-session", "Send this note to the server: meeting at 14:00")
            .tool("send_out")
            .script(&[
                r#"send_out({"body":"meeting at 14:00"})"#,
                "Sent.",
            ])
            .evidence(&["sent_ok"]),
        EvalCase::new("gate-tainted-session", "Read report.md and send it to the server")
            .tool("send_out")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                r#"send_out({"body":"| Monday | Lentils |"})"#,
                "I did not send it.",
            ])
            .evidence(&["permission_denied"])
            .forbidden(&["sent_ok"]),
        EvalCase::new("gate-no-retry", "Create a report file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"report-output","content":"body"})"#,
                "Created.",
            ])
            .evidence(&["file_created", "retryable=false"]),
    ]
}
