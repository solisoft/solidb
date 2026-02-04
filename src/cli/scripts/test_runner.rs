//! Test runner for Lua API tests
//!
//! Discovers and executes test files from the scripts/tests/ directory,
//! testing actual API endpoints against the running server.

use colored::Colorize;
use mlua::{Function, Lua, MultiValue, Result as LuaResult, Table, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

use super::config::Config;
use super::test_http::TestHttpClient;

/// Test result status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed(String),
    Skipped,
}

/// A single test case result
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub describe: String,
    pub status: TestStatus,
    pub duration: Duration,
}

/// Test runner configuration
#[derive(Default)]
pub struct TestRunnerConfig {
    pub verbose: bool,
    pub filter: Option<String>,
    pub file: Option<PathBuf>,
}

/// Summary of test execution
pub struct TestSummary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total_duration: Duration,
    pub results: Vec<TestResult>,
}

impl TestSummary {
    pub fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
            skipped: 0,
            total_duration: Duration::ZERO,
            results: Vec::new(),
        }
    }

    pub fn add(&mut self, result: TestResult) {
        match &result.status {
            TestStatus::Passed => self.passed += 1,
            TestStatus::Failed(_) => self.failed += 1,
            TestStatus::Skipped => self.skipped += 1,
        }
        self.total_duration += result.duration;
        self.results.push(result);
    }
}

impl Default for TestSummary {
    fn default() -> Self {
        Self::new()
    }
}

/// The main test runner
pub struct TestRunner {
    config: Config,
    runner_config: TestRunnerConfig,
    scripts_dir: PathBuf,
}

impl TestRunner {
    /// Create a new test runner
    pub fn new(config: Config, scripts_dir: PathBuf, runner_config: TestRunnerConfig) -> Self {
        Self {
            config,
            runner_config,
            scripts_dir,
        }
    }

    /// Discover test files in the tests/ subdirectory
    pub fn discover_tests(&self) -> Vec<PathBuf> {
        let tests_dir = self.scripts_dir.join("tests");

        if !tests_dir.exists() {
            return Vec::new();
        }

        // If a specific file was requested, only return that
        if let Some(ref file) = self.runner_config.file {
            let file_path = if file.is_absolute() {
                file.clone()
            } else {
                tests_dir.join(file)
            };

            if file_path.exists() {
                return vec![file_path];
            } else {
                return Vec::new();
            }
        }

        let mut files = Vec::new();

        for entry in WalkDir::new(&tests_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Only include .lua files
            if path.is_file() && path.extension().map(|e| e == "lua").unwrap_or(false) {
                files.push(path.to_path_buf());
            }
        }

        files.sort();
        files
    }

    /// Run all discovered tests
    pub fn run(&self) -> anyhow::Result<TestSummary> {
        let test_files = self.discover_tests();

        if test_files.is_empty() {
            println!(
                "{} No test files found in {}",
                "!".yellow(),
                self.scripts_dir.join("tests").display()
            );
            return Ok(TestSummary::new());
        }

        println!(
            "Running tests from {}...\n",
            self.scripts_dir.display()
        );

        let mut summary = TestSummary::new();

        for file in test_files {
            self.run_test_file(&file, &mut summary)?;
        }

        self.print_summary(&summary);
        Ok(summary)
    }

    /// Run a single test file
    fn run_test_file(&self, path: &Path, summary: &mut TestSummary) -> anyhow::Result<()> {
        let code = std::fs::read_to_string(path)?;

        // Create Lua state with test framework
        let lua = Lua::new();
        self.setup_lua(&lua)?;

        // Create HTTP client
        let http_client = TestHttpClient::new(
            &self.config.base_url(),
            &self.config.database,
            &self.config.default_service(),
            self.config.auth_token.clone(),
        );

        // Register HTTP module
        super::test_http::register_http_module(&lua, http_client)?;

        // Collect test results
        let results: Arc<Mutex<Vec<TestResult>>> = Arc::new(Mutex::new(Vec::new()));
        let current_describe: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let verbose = self.runner_config.verbose;
        let filter = self.runner_config.filter.clone();

        // Register describe/it functions
        self.register_test_functions(&lua, results.clone(), current_describe.clone(), verbose, filter)?;

        // Execute the test file
        if let Err(e) = lua.load(&code).exec() {
            eprintln!(
                "{} Error in {}: {}",
                "✗".red(),
                path.file_name().unwrap_or_default().to_string_lossy(),
                e
            );
            return Ok(());
        }

        // Add results to summary
        let results_guard = results.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        for result in results_guard.iter() {
            summary.add(result.clone());
        }

        Ok(())
    }

    /// Setup Lua environment with test utilities
    fn setup_lua(&self, lua: &Lua) -> LuaResult<()> {
        let globals = lua.globals();

        // Remove unsafe globals
        globals.set("os", Value::Nil)?;
        globals.set("io", Value::Nil)?;
        globals.set("debug", Value::Nil)?;
        globals.set("loadfile", Value::Nil)?;
        globals.set("dofile", Value::Nil)?;

        // Add JSON module
        self.register_json_module(lua)?;

        // Add print function (stores output for verbose mode)
        let print_fn = lua.create_function(|_, args: MultiValue| {
            let parts: Vec<String> = args
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_else(|_| "[invalid string]".to_string()),
                    Value::Number(n) => n.to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Nil => "nil".to_string(),
                    Value::Table(_) => "[table]".to_string(),
                    _ => format!("{:?}", v),
                })
                .collect();
            println!("    {}", parts.join("\t").dimmed());
            Ok(())
        })?;
        globals.set("print", print_fn)?;

        Ok(())
    }

    /// Register JSON encode/decode functions
    fn register_json_module(&self, lua: &Lua) -> LuaResult<()> {
        let globals = lua.globals();
        let json = lua.create_table()?;

        // json.encode
        let encode = lua.create_function(|lua, value: Value| {
            let json_value = lua_to_json(lua, &value)?;
            serde_json::to_string(&json_value)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
        })?;
        json.set("encode", encode)?;

        // json.decode
        let decode = lua.create_function(|lua, s: String| {
            let json_value: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            json_to_lua(lua, &json_value)
        })?;
        json.set("decode", decode)?;

        globals.set("json", json)?;
        Ok(())
    }

    /// Register describe/it/expect functions
    fn register_test_functions(
        &self,
        lua: &Lua,
        results: Arc<Mutex<Vec<TestResult>>>,
        current_describe: Arc<Mutex<String>>,
        _verbose: bool,
        filter: Option<String>,
    ) -> LuaResult<()> {
        let globals = lua.globals();

        // describe(name, fn)
        let describe_current = Arc::clone(&current_describe);
        let describe_filter = filter.clone();
        let describe_fn = lua.create_function(move |lua, (name, func): (String, Function)| {
            // Check filter
            if let Some(ref f) = describe_filter {
                if !name.to_lowercase().contains(&f.to_lowercase()) {
                    return Ok(());
                }
            }

            println!("{}", name);
            if let Ok(mut current) = describe_current.lock() {
                *current = name;
            }

            // Create hooks storage
            let hooks: Table = lua.create_table()?;
            hooks.set("before", Value::Nil)?;
            hooks.set("after", Value::Nil)?;
            hooks.set("before_all", Value::Nil)?;
            hooks.set("after_all", Value::Nil)?;
            lua.globals().set("__test_hooks", hooks)?;

            // Run the describe block
            func.call::<()>(())?;

            // Clear hooks
            lua.globals().set("__test_hooks", Value::Nil)?;

            println!();
            Ok(())
        })?;
        globals.set("describe", describe_fn)?;

        // it(name, fn)
        let it_results = Arc::clone(&results);
        let it_current = Arc::clone(&current_describe);
        let it_fn = lua.create_function(move |lua, (name, func): (String, Function)| {
            let describe_name = it_current.lock()
                .map(|guard| guard.clone())
                .unwrap_or_default();
            let start = Instant::now();

            // Run before hook
            if let Ok(hooks) = lua.globals().get::<Table>("__test_hooks") {
                if let Ok(before) = hooks.get::<Function>("before") {
                    let _ = before.call::<()>(());
                }
            }

            // Run the test
            let result = func.call::<()>(());
            let duration = start.elapsed();

            // Run after hook
            if let Ok(hooks) = lua.globals().get::<Table>("__test_hooks") {
                if let Ok(after) = hooks.get::<Function>("after") {
                    let _ = after.call::<()>(());
                }
            }

            let status = match result {
                Ok(_) => {
                    println!(
                        "  {} {} {}",
                        "✓".green(),
                        name,
                        format!("({}ms)", duration.as_millis()).dimmed()
                    );
                    TestStatus::Passed
                }
                Err(e) => {
                    let msg = e.to_string();
                    println!(
                        "  {} {} {}",
                        "✗".red(),
                        name,
                        format!("({}ms)", duration.as_millis()).dimmed()
                    );
                    println!("    {}", msg.red());
                    TestStatus::Failed(msg)
                }
            };

            if let Ok(mut results) = it_results.lock() {
                results.push(TestResult {
                    name,
                    describe: describe_name,
                    status,
                    duration,
                });
            }

            Ok(())
        })?;
        globals.set("it", it_fn)?;

        // before(fn) - run before each test
        let before_fn = lua.create_function(|lua, func: Function| {
            if let Ok(hooks) = lua.globals().get::<Table>("__test_hooks") {
                hooks.set("before", func)?;
            }
            Ok(())
        })?;
        globals.set("before", before_fn)?;

        // after(fn) - run after each test
        let after_fn = lua.create_function(|lua, func: Function| {
            if let Ok(hooks) = lua.globals().get::<Table>("__test_hooks") {
                hooks.set("after", func)?;
            }
            Ok(())
        })?;
        globals.set("after", after_fn)?;

        // before_all(fn) - run once before all tests
        let before_all_fn = lua.create_function(|_lua, func: Function| {
            // Execute immediately
            func.call::<()>(())?;
            Ok(())
        })?;
        globals.set("before_all", before_all_fn)?;

        // after_all(fn) - run once after all tests
        let after_all_fn = lua.create_function(|lua, func: Function| {
            if let Ok(hooks) = lua.globals().get::<Table>("__test_hooks") {
                hooks.set("after_all", func)?;
            }
            Ok(())
        })?;
        globals.set("after_all", after_all_fn)?;

        // expect(value) - returns expectation object
        self.register_expect(lua)?;

        Ok(())
    }

    /// Register the expect function and matchers
    fn register_expect(&self, lua: &Lua) -> LuaResult<()> {
        let globals = lua.globals();

        let expect_fn = lua.create_function(|lua, value: Value| {
            let expectation = lua.create_table()?;
            expectation.set("__value", value.clone())?;

            // to_equal(expected)
            let to_equal = lua.create_function(|lua, (this, expected): (Table, Value)| {
                let actual: Value = this.get("__value")?;
                if !values_equal(&actual, &expected) {
                    let actual_str = value_to_string(lua, &actual)?;
                    let expected_str = value_to_string(lua, &expected)?;
                    return Err(mlua::Error::RuntimeError(format!(
                        "Expected {} to equal {}",
                        actual_str, expected_str
                    )));
                }
                Ok(())
            })?;
            expectation.set("to_equal", to_equal)?;

            // to_exist()
            let to_exist = lua.create_function(|_, this: Table| {
                let actual: Value = this.get("__value")?;
                if matches!(actual, Value::Nil) {
                    return Err(mlua::Error::RuntimeError(
                        "Expected value to exist, got nil".to_string(),
                    ));
                }
                Ok(())
            })?;
            expectation.set("to_exist", to_exist)?;

            // to_be_nil()
            let to_be_nil = lua.create_function(|lua, this: Table| {
                let actual: Value = this.get("__value")?;
                if !matches!(actual, Value::Nil) {
                    let actual_str = value_to_string(lua, &actual)?;
                    return Err(mlua::Error::RuntimeError(format!(
                        "Expected nil, got {}",
                        actual_str
                    )));
                }
                Ok(())
            })?;
            expectation.set("to_be_nil", to_be_nil)?;

            // to_contain(substring)
            let to_contain = lua.create_function(|_, (this, substring): (Table, String)| {
                let actual: Value = this.get("__value")?;
                match actual {
                    Value::String(s) => {
                        let s = s.to_str()?;
                        if !s.contains(&substring) {
                            return Err(mlua::Error::RuntimeError(format!(
                                "Expected '{}' to contain '{}'",
                                s, substring
                            )));
                        }
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "to_contain can only be used with strings".to_string(),
                        ));
                    }
                }
                Ok(())
            })?;
            expectation.set("to_contain", to_contain)?;

            // to_match(pattern)
            let to_match = lua.create_function(|_, (this, pattern): (Table, String)| {
                let actual: Value = this.get("__value")?;
                match actual {
                    Value::String(s) => {
                        let s_str = s.to_str()?.to_string();
                        let re = regex::Regex::new(&pattern)
                            .map_err(|e| mlua::Error::RuntimeError(format!("Invalid regex: {}", e)))?;
                        if !re.is_match(&s_str) {
                            return Err(mlua::Error::RuntimeError(format!(
                                "Expected '{}' to match pattern '{}'",
                                s_str, pattern
                            )));
                        }
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "to_match can only be used with strings".to_string(),
                        ));
                    }
                }
                Ok(())
            })?;
            expectation.set("to_match", to_match)?;

            // to_be_greater_than(n)
            let to_be_greater_than = lua.create_function(|_, (this, n): (Table, f64)| {
                let actual: Value = this.get("__value")?;
                let actual_num = match actual {
                    Value::Number(num) => num,
                    Value::Integer(i) => i as f64,
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "to_be_greater_than can only be used with numbers".to_string(),
                        ));
                    }
                };
                if actual_num <= n {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Expected {} to be greater than {}",
                        actual_num, n
                    )));
                }
                Ok(())
            })?;
            expectation.set("to_be_greater_than", to_be_greater_than)?;

            // to_be_less_than(n)
            let to_be_less_than = lua.create_function(|_, (this, n): (Table, f64)| {
                let actual: Value = this.get("__value")?;
                let actual_num = match actual {
                    Value::Number(num) => num,
                    Value::Integer(i) => i as f64,
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "to_be_less_than can only be used with numbers".to_string(),
                        ));
                    }
                };
                if actual_num >= n {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Expected {} to be less than {}",
                        actual_num, n
                    )));
                }
                Ok(())
            })?;
            expectation.set("to_be_less_than", to_be_less_than)?;

            // to_throw()
            let to_throw = lua.create_function(|_, this: Table| {
                let func: Value = this.get("__value")?;
                match func {
                    Value::Function(f) => {
                        let result: LuaResult<()> = f.call(());
                        if result.is_ok() {
                            return Err(mlua::Error::RuntimeError(
                                "Expected function to throw, but it did not".to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "to_throw can only be used with functions".to_string(),
                        ));
                    }
                }
                Ok(())
            })?;
            expectation.set("to_throw", to_throw)?;

            // to_be_true()
            let to_be_true = lua.create_function(|_, this: Table| {
                let actual: Value = this.get("__value")?;
                match actual {
                    Value::Boolean(true) => Ok(()),
                    Value::Boolean(false) => Err(mlua::Error::RuntimeError(
                        "Expected true, got false".to_string(),
                    )),
                    _ => Err(mlua::Error::RuntimeError(
                        "Expected true, got non-boolean".to_string(),
                    )),
                }
            })?;
            expectation.set("to_be_true", to_be_true)?;

            // to_be_false()
            let to_be_false = lua.create_function(|_, this: Table| {
                let actual: Value = this.get("__value")?;
                match actual {
                    Value::Boolean(false) => Ok(()),
                    Value::Boolean(true) => Err(mlua::Error::RuntimeError(
                        "Expected false, got true".to_string(),
                    )),
                    _ => Err(mlua::Error::RuntimeError(
                        "Expected false, got non-boolean".to_string(),
                    )),
                }
            })?;
            expectation.set("to_be_false", to_be_false)?;

            Ok(expectation)
        })?;
        globals.set("expect", expect_fn)?;

        Ok(())
    }

    /// Print the test summary
    fn print_summary(&self, summary: &TestSummary) {
        println!("{}", "─".repeat(50));

        let status_line = if summary.failed > 0 {
            format!(
                "Tests: {} passed, {} failed",
                summary.passed.to_string().green(),
                summary.failed.to_string().red()
            )
        } else {
            format!(
                "Tests: {} passed",
                summary.passed.to_string().green()
            )
        };

        if summary.skipped > 0 {
            println!("{}, {} skipped", status_line, summary.skipped);
        } else {
            println!("{}", status_line);
        }

        println!("Time:  {}ms", summary.total_duration.as_millis());
    }
}

/// Check if two Lua values are equal
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => {
            ((*a as f64) - b).abs() < f64::EPSILON
        }
        (Value::String(a), Value::String(b)) => {
            a.to_str().ok() == b.to_str().ok()
        }
        (Value::Table(a), Value::Table(b)) => {
            // Simple table comparison - check all keys
            let a_pairs: Vec<_> = a.pairs::<Value, Value>().filter_map(|r| r.ok()).collect();
            let b_pairs: Vec<_> = b.pairs::<Value, Value>().filter_map(|r| r.ok()).collect();
            if a_pairs.len() != b_pairs.len() {
                return false;
            }
            for (key, val) in a_pairs {
                match b.get::<Value>(key.clone()) {
                    Ok(b_val) => {
                        if !values_equal(&val, &b_val) {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
            true
        }
        _ => false,
    }
}

/// Convert Lua value to string for display
fn value_to_string(lua: &Lua, value: &Value) -> LuaResult<String> {
    match value {
        Value::Nil => Ok("nil".to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::String(s) => Ok(format!("\"{}\"", s.to_str()?)),
        Value::Table(_) => {
            let json = lua_to_json(lua, value)?;
            Ok(serde_json::to_string(&json).unwrap_or_else(|_| "[table]".to_string()))
        }
        Value::Function(_) => Ok("[function]".to_string()),
        _ => Ok(format!("{:?}", value)),
    }
}

/// Convert Lua value to JSON
#[allow(clippy::only_used_in_recursion)]
fn lua_to_json(lua: &Lua, value: &Value) -> LuaResult<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number((*i).into())),
        Value::Number(n) => {
            serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .ok_or_else(|| mlua::Error::RuntimeError("Invalid number".to_string()))
        }
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(t) => {
            // Check if it's an array (sequential integer keys starting from 1)
            let len = t.len()?;
            if len > 0 {
                let mut is_array = true;
                for i in 1..=len {
                    if t.get::<Value>(i).is_err() {
                        is_array = false;
                        break;
                    }
                }
                if is_array {
                    let mut arr = Vec::new();
                    for i in 1..=len {
                        let val: Value = t.get(i)?;
                        arr.push(lua_to_json(lua, &val)?);
                    }
                    return Ok(serde_json::Value::Array(arr));
                }
            }

            // It's an object
            let mut map = serde_json::Map::new();
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                let key = match k {
                    Value::String(s) => s.to_str()?.to_string(),
                    Value::Integer(i) => i.to_string(),
                    _ => continue,
                };
                map.insert(key, lua_to_json(lua, &v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Ok(serde_json::Value::Null),
    }
}

/// Convert JSON to Lua value
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> LuaResult<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}
