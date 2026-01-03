-- SoliDB AI Agent Example
-- This script demonstrates the AI contribution pipeline from Lua scripts
--
-- The AI pipeline allows natural language contributions to be processed
-- through analysis, code generation, validation, and review stages.

-- Register a new AI agent
function register_analyzer_agent()
    local agent = solidb.ai.register_agent(
        "lua-analyzer-001",
        "analyzer",
        {"rust", "lua", "documentation"}
    )

    return {
        message = "Agent registered successfully",
        agent = agent
    }
end

-- Submit a feature contribution request
function submit_feature_request()
    local contribution_id = solidb.ai.submit_contribution(
        "feature",
        "Add a new CONTAINS() function to SDBQL that checks if an array contains a value",
        {
            related_collections = {"_system_functions", "sdbql_docs"},
            priority = "medium"
        }
    )

    return {
        message = "Feature contribution submitted",
        contribution_id = contribution_id
    }
end

-- Submit a bugfix contribution request
function submit_bugfix_request()
    local contribution_id = solidb.ai.submit_contribution(
        "bugfix",
        "Fix edge case where SORT with null values causes panic",
        {
            priority = "high"
        }
    )

    return {
        message = "Bugfix contribution submitted",
        contribution_id = contribution_id
    }
end

-- Get contribution details
function get_contribution_details(contribution_id)
    local contribution = solidb.ai.get_contribution(contribution_id)

    if not contribution then
        return {
            error = "Contribution not found",
            contribution_id = contribution_id
        }
    end

    return {
        contribution = contribution,
        current_status = contribution.status,
        requires_review = contribution.risk_score and contribution.risk_score > 0.7
    }
end

-- List all pending contributions
function list_pending_contributions()
    local contributions = solidb.ai.list_contributions({
        status = "Submitted",
        limit = 10
    })

    return {
        count = #contributions,
        contributions = contributions
    }
end

-- List contributions in review
function list_contributions_in_review()
    local contributions = solidb.ai.list_contributions({
        status = "Review",
        limit = 20
    })

    return {
        count = #contributions,
        contributions = contributions
    }
end

-- Get pending tasks for this agent to work on
function get_my_pending_tasks(task_type)
    local options = { limit = 5 }
    if task_type then
        options.task_type = task_type
    end

    local tasks = solidb.ai.get_pending_tasks(options)

    return {
        count = #tasks,
        tasks = tasks
    }
end

-- Claim and process an analysis task
function process_analysis_task(task_id, agent_id)
    -- First claim the task
    local task = solidb.ai.claim_task(task_id, agent_id)

    -- Perform analysis (simulated)
    local analysis_result = {
        risk_score = 0.3,
        requires_review = false,
        affected_files = {
            "src/sdbql/functions.rs",
            "tests/sdbql_function_tests.rs"
        },
        complexity = "low",
        estimated_changes = 2,
        dependencies = {},
        recommendations = {
            "Add function to existing functions.rs",
            "Add corresponding tests",
            "Update documentation"
        }
    }

    -- Complete the task with our analysis
    local result = solidb.ai.complete_task(task_id, analysis_result)

    return {
        message = result.message,
        next_tasks_created = result.next_tasks_created,
        completed_task = result.task
    }
end

-- Claim and process a code generation task
function process_generation_task(task_id, agent_id)
    -- Claim the task
    local task = solidb.ai.claim_task(task_id, agent_id)

    -- Generate code (simulated)
    local generation_result = {
        files = {
            {
                path = "src/sdbql/functions.rs",
                action = "modify",
                content = [[
/// Check if array contains a value
fn contains(args: Vec<Value>) -> Result<Value, Error> {
    if args.len() != 2 {
        return Err(Error::InvalidArguments("CONTAINS requires 2 arguments"));
    }
    let array = args[0].as_array()?;
    let needle = &args[1];
    Ok(Value::Bool(array.contains(needle)))
}
]]
            },
            {
                path = "tests/sdbql_function_tests.rs",
                action = "modify",
                content = [[
#[test]
fn test_contains_function() {
    let result = execute_query("RETURN CONTAINS([1, 2, 3], 2)");
    assert_eq!(result, json!(true));

    let result = execute_query("RETURN CONTAINS([1, 2, 3], 5)");
    assert_eq!(result, json!(false));
}
]]
            }
        },
        summary = "Added CONTAINS() function with tests"
    }

    -- Complete the task
    local result = solidb.ai.complete_task(task_id, generation_result)

    return {
        message = result.message,
        next_tasks_created = result.next_tasks_created
    }
end

-- Claim and process a validation task
function process_validation_task(task_id, agent_id)
    -- Claim the task
    local task = solidb.ai.claim_task(task_id, agent_id)

    -- Validate code (simulated - would run actual validation)
    local validation_result = {
        passed = true,
        stages = {
            { name = "syntax", passed = true, message = "rustfmt check passed" },
            { name = "linting", passed = true, message = "clippy check passed" },
            { name = "type_check", passed = true, message = "cargo check passed" }
        }
    }

    -- Complete the task
    local result = solidb.ai.complete_task(task_id, validation_result)

    return {
        message = result.message,
        validation_passed = validation_result.passed
    }
end

-- Agent worker loop - continuously process tasks
function agent_worker_loop(agent_id, agent_type)
    local processed = 0
    local max_iterations = 10

    for i = 1, max_iterations do
        -- Get pending tasks for our agent type
        local task_type_map = {
            analyzer = "AnalyzeContribution",
            coder = "GenerateCode",
            tester = "RunTests",
            reviewer = "ValidateCode"
        }

        local task_type = task_type_map[agent_type]
        local tasks = solidb.ai.get_pending_tasks({
            task_type = task_type,
            limit = 1
        })

        if #tasks == 0 then
            break -- No more tasks
        end

        local task = tasks[1]

        -- Process based on task type
        if task_type == "AnalyzeContribution" then
            process_analysis_task(task.id, agent_id)
        elseif task_type == "GenerateCode" then
            process_generation_task(task.id, agent_id)
        elseif task_type == "ValidateCode" then
            process_validation_task(task.id, agent_id)
        end

        processed = processed + 1
    end

    return {
        agent_id = agent_id,
        tasks_processed = processed,
        status = "completed"
    }
end

-- Dashboard view - show current pipeline status
function pipeline_dashboard()
    local submitted = solidb.ai.list_contributions({ status = "Submitted", limit = 100 })
    local analyzing = solidb.ai.list_contributions({ status = "Analyzing", limit = 100 })
    local generating = solidb.ai.list_contributions({ status = "Generating", limit = 100 })
    local validating = solidb.ai.list_contributions({ status = "Validating", limit = 100 })
    local review = solidb.ai.list_contributions({ status = "Review", limit = 100 })
    local approved = solidb.ai.list_contributions({ status = "Approved", limit = 100 })
    local merged = solidb.ai.list_contributions({ status = "Merged", limit = 100 })

    local pending_tasks = solidb.ai.get_pending_tasks({ limit = 100 })

    return {
        pipeline_status = {
            submitted = #submitted,
            analyzing = #analyzing,
            generating = #generating,
            validating = #validating,
            review = #review,
            approved = #approved,
            merged = #merged
        },
        pending_tasks = #pending_tasks,
        recent_contributions = submitted
    }
end

-- Main handler - route based on path
if request.path:match("register") then
    return register_analyzer_agent()
elseif request.path:match("submit/feature") then
    return submit_feature_request()
elseif request.path:match("submit/bugfix") then
    return submit_bugfix_request()
elseif request.path:match("contribution/(.+)") then
    local id = request.path:match("contribution/(.+)")
    return get_contribution_details(id)
elseif request.path:match("pending") then
    return list_pending_contributions()
elseif request.path:match("review") then
    return list_contributions_in_review()
elseif request.path:match("tasks") then
    local task_type = request.query and request.query.type
    return get_my_pending_tasks(task_type)
elseif request.path:match("work") then
    local agent_id = request.query and request.query.agent or "lua-worker-001"
    local agent_type = request.query and request.query.type or "analyzer"
    return agent_worker_loop(agent_id, agent_type)
elseif request.path:match("dashboard") then
    return pipeline_dashboard()
else
    return response.json({
        title = "SoliDB AI Contribution Pipeline",
        description = "AI agents working alongside humans to improve SoliDB",
        endpoints = {
            ["/register"] = "Register a new AI analyzer agent",
            ["/submit/feature"] = "Submit a feature request",
            ["/submit/bugfix"] = "Submit a bugfix request",
            ["/contribution/{id}"] = "Get contribution details",
            ["/pending"] = "List pending contributions",
            ["/review"] = "List contributions awaiting review",
            ["/tasks?type=X"] = "Get pending tasks (AnalyzeContribution, GenerateCode, etc.)",
            ["/work?agent=X&type=analyzer"] = "Run agent worker loop",
            ["/dashboard"] = "View pipeline status dashboard"
        },
        pipeline_stages = {
            "1. Submitted - Natural language request received",
            "2. Analyzing - AI analyzes scope and risk",
            "3. Generating - AI generates code changes",
            "4. Validating - Code passes syntax, lint, type checks",
            "5. Testing - Unit and integration tests run",
            "6. Review - Human review for high-risk changes",
            "7. Approved - Ready for merge",
            "8. Merged - Changes integrated into codebase"
        }
    })
end
