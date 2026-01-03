import os
import time
import requests
import json

# Configuration
SOLIdB_URL = os.getenv("SOLIDB_URL", "http://localhost:8080/_api/database/default")
SOLIDB_KEY = os.getenv("SOLIDB_KEY", "admin_secret_key")
ANTHROPIC_KEY = os.getenv("ANTHROPIC_KEY")

if not ANTHROPIC_KEY:
    print("❌ Error: ANTHROPIC_KEY environment variable not set")
    exit(1)

# Headers
db_headers = {
    "Authorization": f"Bearer {SOLIDB_KEY}",
    "Content-Type": "application/json"
}

claude_headers = {
    "x-api-key": ANTHROPIC_KEY,
    "anthropic-version": "2023-06-01",
    "content-type": "application/json"
}

AGENT_NAME = "Claude-Opus-Worker-01"

def register():
    """Register this process as an agent in SoliDB"""
    print(f"🔌 Connecting to SoliDB at {SOLIdB_URL}...")
    try:
        payload = {
            "name": AGENT_NAME,
            "agent_type": "architect", # Architect agents plan and design
            "capabilities": ["code-generation", "architectural-analysis", "claude-3-opus"]
        }
        resp = requests.post(f"{SOLIdB_URL}/ai/agents", headers=db_headers, json=payload)
        resp.raise_for_status()
        agent = resp.json()
        print(f"✅ Registered Agent ID: {agent['id']}")
        return agent['id']
    except Exception as e:
        print(f"❌ Registration failed: {e}")
        return None

def call_claude(prompt, system_prompt="You are a helpful AI assistant."):
    """Send request to Anthropic API"""
    print("🧠 Thinking (Claude 3 Opus)...")
    
    payload = {
        "model": "claude-3-opus-20240229",
        "max_tokens": 4096,
        "system": system_prompt,
        "messages": [
            {"role": "user", "content": prompt}
        ]
    }
    
    try:
        resp = requests.post("https://api.anthropic.com/v1/messages", headers=claude_headers, json=payload)
        resp.raise_for_status()
        data = resp.json()
        return data['content'][0]['text']
    except Exception as e:
        print(f"❌ Claude API Error: {e}")
        if hasattr(e, 'response') and e.response:
            print(e.response.text)
        return None

def process_tasks(agent_id):
    """Main event loop"""
    print("🚀 Agent started. Waiting for tasks...")
    
    while True:
        try:
            # 1. Send Heartbeat
            requests.post(f"{SOLIdB_URL}/ai/agents/{agent_id}/heartbeat", headers=db_headers)

            # 2. Poll for pending tasks (specifically looking for analysis or code generation)
            resp = requests.get(f"{SOLIdB_URL}/ai/tasks?status=pending", headers=db_headers)
            
            if resp.status_code == 200:
                tasks = resp.json().get('tasks', [])
                for task in tasks:
                    # Filter only tasks we care about
                    task_type = task['task_type']
                    if task_type not in ["analyze_contribution", "generate_code"]:
                        continue

                    print(f"📥 Found task: {task['id']} ({task_type})")
                    
                    # 3. Claim Task
                    claim = requests.post(
                        f"{SOLIdB_URL}/ai/tasks/{task['id']}/claim", 
                        headers=db_headers, 
                        json={"agent_id": agent_id}
                    )
                    
                    if claim.status_code == 200:
                        # 4. Process with Claude
                        task_input = task.get('input', {})
                        
                        # Determine prompt based on task type
                        prompt = ""
                        system = "You are a senior software engineer."
                        
                        if task_type == "analyze_contribution":
                            desc = task_input.get('description', 'No description')
                            prompt = f"Analyze this feature request and provide an implementation plan:\n\n{desc}"
                            system += " Output JSON with 'plan', 'files_to_change', and 'risk_score'."
                        
                        elif task_type == "generate_code":
                            plan = task_input.get('plan', 'No plan')
                            prompt = f"Generate the code according to this plan:\n\n{plan}"
                            system += " Output the complete code files."

                        # Call AI
                        result_text = call_claude(prompt, system)
                        
                        if result_text:
                            # 5. Complete Task
                            # Try to parse JSON if Claude returned it, otherwise wrap text
                            try:
                                output_data = json.loads(result_text)
                            except:
                                output_data = {"raw_output": result_text}

                            requests.post(
                                f"{SOLIdB_URL}/ai/tasks/{task['id']}/complete", 
                                headers=db_headers, 
                                json={"output": output_data}
                            )
                            print(f"✅ Task {task['id']} completed successfully!")
                        else:
                             requests.post(
                                f"{SOLIdB_URL}/ai/tasks/{task['id']}/fail", 
                                headers=db_headers, 
                                json={"error": "AI Provider failed"}
                            )
                            
            time.sleep(2)
            
        except KeyboardInterrupt:
            break
        except Exception as e:
            print(f"⚠️ Loop error: {e}")
            time.sleep(5)

if __name__ == "__main__":
    aid = register()
    if aid:
        try:
            process_tasks(aid)
        except KeyboardInterrupt:
            print("\n👋 Shutting down agent")
            requests.delete(f"{SOLIdB_URL}/ai/agents/{aid}", headers=db_headers)
