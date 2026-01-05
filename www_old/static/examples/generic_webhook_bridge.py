import os
import time
import requests
import json

# ==============================================================================
# GENERIC WEBHOOK BRIDGE
# 
# This script bridges SoliDB's Polling Architecture to your Webhook Architecture.
# 1. It Polls SoliDB for pending tasks.
# 2. It Pushes (POST) the task to YOUR_WEBHOOK_URL.
# 3. It returns the response back to SoliDB.
# ==============================================================================

# Configuration
SOLIDB_URL = os.getenv("SOLIDB_URL", "http://localhost:8080/_api/database/default")
SOLIDB_KEY = os.getenv("SOLIDB_KEY", "admin_secret_key")

# The remote URL where your Agent lives (e.g., a Vercel function, Flask app)
YOUR_WEBHOOK_URL = os.getenv("WEBHOOK_URL", "http://localhost:3000/api/my-agent")
YOUR_WEBHOOK_SECRET = os.getenv("WEBHOOK_SECRET", "my-secret-token")

AGENT_NAME = os.getenv("AGENT_NAME", "Webhook-Bridge-01")

# Headers
db_headers = {
    "Authorization": f"Bearer {SOLIDB_KEY}",
    "Content-Type": "application/json"
}

webhook_headers = {
    "Content-Type": "application/json",
    "X-Agent-Secret": YOUR_WEBHOOK_SECRET
}

def register():
    print(f"🔌 Connecting Bridge to SoliDB: {SOLIDB_URL}")
    print(f"🔗 Forwarding tasks to Webhook: {YOUR_WEBHOOK_URL}")
    
    try:
        payload = {
            "name": AGENT_NAME,
            "agent_type": "generic",
            "capabilities": ["webhook-proxy"]
        }
        resp = requests.post(f"{SOLIDB_URL}/ai/agents", headers=db_headers, json=payload)
        resp.raise_for_status()
        agent = resp.json()
        print(f"✅ Bridge Registered. ID: {agent['id']}")
        return agent['id']
    except Exception as e:
        print(f"❌ Registration failed: {e}")
        return None

def process_bridge(agent_id):
    print("🚀 Bridge is active. Waiting for tasks...")
    
    while True:
        try:
            # 1. Heartbeat
            requests.post(f"{SOLIDB_URL}/ai/agents/{agent_id}/heartbeat", headers=db_headers)

            # 2. Poll
            resp = requests.get(f"{SOLIDB_URL}/ai/tasks?status=pending", headers=db_headers)
            
            if resp.status_code == 200:
                tasks = resp.json().get('tasks', [])
                for task in tasks:
                    print(f"📥 Received Task: {task['id']} -> Forwarding to Webhook...")
                    
                    # 3. Claim
                    requests.post(f"{SOLIDB_URL}/ai/tasks/{task['id']}/claim", 
                                headers=db_headers, json={"agent_id": agent_id})
                    
                    # 4. FORWARD TO WEBHOOK (The "Push")
                    try:
                        hook_resp = requests.post(
                            YOUR_WEBHOOK_URL, 
                            headers=webhook_headers, 
                            json=task, 
                            timeout=300
                        )
                        hook_resp.raise_for_status()
                        result = hook_resp.json()
                        
                        # 5. Complete
                        requests.post(f"{SOLIDB_URL}/ai/tasks/{task['id']}/complete", 
                                    headers=db_headers, json={"output": result})
                        print(f"✅ Webhook Success! Task {task['id']} completed.")
                        
                    except Exception as he:
                        print(f"⚠️ Webhook Failed: {he}")
                        # Report failure back to DB
                        requests.post(f"{SOLIDB_URL}/ai/tasks/{task['id']}/fail", 
                                    headers=db_headers, json={"error": str(he)})
                            
            time.sleep(2)
            
        except KeyboardInterrupt:
            break
        except Exception as e:
            print(f"⚠️ Bridge error: {e}")
            time.sleep(5)

if __name__ == "__main__":
    aid = register()
    if aid:
        try:
            process_bridge(aid)
        except KeyboardInterrupt:
            requests.delete(f"{SOLIDB_URL}/ai/agents/{aid}", headers=db_headers)
