import os
import time
import requests
import json

# Configuration
SOLIDB_URL = os.getenv("SOLIDB_URL", "http://localhost:8080/_api/database/default")
SOLIDB_KEY = os.getenv("SOLIDB_KEY", "admin_secret_key")
XAI_KEY = os.getenv("XAI_KEY")

if not XAI_KEY:
    print("❌ Error: XAI_KEY environment variable not set")
    exit(1)

# Headers
db_headers = {
    "Authorization": f"Bearer {SOLIDB_KEY}",
    "Content-Type": "application/json"
}

grok_headers = {
    "Authorization": f"Bearer {XAI_KEY}",
    "Content-Type": "application/json"
}

AGENT_NAME = "Grok-Worker-01"

def register():
    print(f"🔌 Connecting to SoliDB at {SOLIDB_URL}...")
    try:
        payload = {
            "name": AGENT_NAME,
            "agent_type": "analyzer", 
            "capabilities": ["analysis", "humor", "grok-1"]
        }
        resp = requests.post(f"{SOLIDB_URL}/ai/agents", headers=db_headers, json=payload)
        resp.raise_for_status()
        agent = resp.json()
        print(f"✅ Registered Agent ID: {agent['id']}")
        return agent['id']
    except Exception as e:
        print(f"❌ Registration failed: {e}")
        return None

def call_grok(prompt, system_prompt="You are Grok, an AI modeled after the Hitchhiker's Guide to the Galaxy."):
    print("🧠 Thinking (Grok)...")
    
    payload = {
        "model": "grok-1", # Adjust model name as per API release
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": prompt}
        ],
        "stream": False
    }
    
    try:
        # Assuming xAI uses OpenAI-compatible endpoint structure
        resp = requests.post("https://api.x.ai/v1/chat/completions", headers=grok_headers, json=payload)
        resp.raise_for_status()
        data = resp.json()
        return data['choices'][0]['message']['content']
    except Exception as e:
        print(f"❌ Grok API Error: {e}")
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
                    # Filter for Analyze tasks which Grok might be used for
                    if task['task_type'] not in ["analyze_contribution", "general_chat"]:
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
                        
                        system_prompt = "You are an expert system analyzer."
                        
                        if task['task_type'] == "analyze_contribution":
                            system_prompt += " Analyze the user's request for potential risks and architectural impact."

                        result_text = call_grok(f"Analyze this:\n{prompt}", system_prompt)
                        
                        if result_text:
                            # 5. Complete
                            requests.post(
                                f"{SOLIDB_URL}/ai/tasks/{task['id']}/complete", 
                                headers=db_headers, 
                                json={"output": {"analysis": result_text}}
                            )
                            print(f"✅ Task {task['id']} completed!")
                        else:
                             requests.post(
                                f"{SOLIDB_URL}/ai/tasks/{task['id']}/fail", 
                                headers=db_headers, 
                                json={"error": "AI Provider (Grok) failed"}
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
