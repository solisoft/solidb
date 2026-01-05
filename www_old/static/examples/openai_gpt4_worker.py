import os
import time
import requests
import json

# Configuration
SOLIDB_URL = os.getenv("SOLIDB_URL", "http://localhost:8080/_api/database/default")
SOLIDB_KEY = os.getenv("SOLIDB_KEY", "admin_secret_key")
OPENAI_KEY = os.getenv("OPENAI_KEY")

if not OPENAI_KEY:
    print("❌ Error: OPENAI_KEY environment variable not set")
    exit(1)

# Headers
db_headers = {
    "Authorization": f"Bearer {SOLIDB_KEY}",
    "Content-Type": "application/json"
}

openai_headers = {
    "Authorization": f"Bearer {OPENAI_KEY}",
    "Content-Type": "application/json"
}

AGENT_NAME = "GPT4-Worker-01"

def register():
    print(f"🔌 Connecting to SoliDB at {SOLIDB_URL}...")
    try:
        payload = {
            "name": AGENT_NAME,
            "agent_type": "coder", 
            "capabilities": ["python", "rust", "code-generation", "gpt-4"]
        }
        resp = requests.post(f"{SOLIDB_URL}/ai/agents", headers=db_headers, json=payload)
        resp.raise_for_status()
        agent = resp.json()
        print(f"✅ Registered Agent ID: {agent['id']}")
        return agent['id']
    except Exception as e:
        print(f"❌ Registration failed: {e}")
        return None

def call_openai(prompt, system_prompt="You are a helpful AI assistant."):
    print("🧠 Thinking (GPT-4)...")
    
    payload = {
        "model": "gpt-4-turbo-preview",
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.2
    }
    
    try:
        resp = requests.post("https://api.openai.com/v1/chat/completions", headers=openai_headers, json=payload)
        resp.raise_for_status()
        data = resp.json()
        return data['choices'][0]['message']['content']
    except Exception as e:
        print(f"❌ OpenAI API Error: {e}")
        if hasattr(e, 'response') and e.response:
            print(e.response.text)
        return None

def process_tasks(agent_id):
    print(f"🚀 {AGENT_NAME} started. Waiting for tasks...")
    
    while True:
        try:
            # 1. Heartbeat
            requests.post(f"{SOLIDB_URL}/ai/agents/{agent_id}/heartbeat", headers=db_headers)

            # 2. Poll
            resp = requests.get(f"{SOLIDB_URL}/ai/tasks?status=pending", headers=db_headers)
            
            if resp.status_code == 200:
                tasks = resp.json().get('tasks', [])
                for task in tasks:
                    # Filter for Coding tasks which GPT-4 is good at
                    if task['task_type'] not in ["generate_code", "refactor_code", "write_tests"]:
                        continue

                    print(f"📥 Found task: {task['id']} ({task['task_type']})")
                    
                    # 3. Claim
                    claim = requests.post(
                        f"{SOLIDB_URL}/ai/tasks/{task['id']}/claim", 
                        headers=db_headers, 
                        json={"agent_id": agent_id}
                    )
                    
                    if claim.status_code == 200:
                        # 4. Process
                        task_input = task.get('input', {})
                        prompt = json.dumps(task_input, indent=2)
                        
                        system_prompts = {
                            "generate_code": "You are a senior Rust/Python developer. Output ONLY valid source code based on the JSON spec provided.",
                            "refactor_code": "Refactor the following code for performance and readability.",
                            "write_tests": "Write comprehensive unit tests for the provided code."
                        }
                        
                        sys_prompt = system_prompts.get(task['task_type'], "You are a helpful coding assistant.")

                        result_text = call_openai(f"Task Input:\n{prompt}", sys_prompt)
                        
                        if result_text:
                            # 5. Complete
                            requests.post(
                                f"{SOLIDB_URL}/ai/tasks/{task['id']}/complete", 
                                headers=db_headers, 
                                json={"output": {"response": result_text}}
                            )
                            print(f"✅ Task {task['id']} completed!")
                        else:
                             requests.post(
                                f"{SOLIDB_URL}/ai/tasks/{task['id']}/fail", 
                                headers=db_headers, 
                                json={"error": "SoliDB: AI Provider failed"}
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
            requests.delete(f"{SOLIDB_URL}/ai/agents/{aid}", headers=db_headers)
