import * as vscode from 'vscode';

// Activation function
export function activate(context: vscode.ExtensionContext) {
    console.log('SoliDB extension is now active');

    // Register completion provider for Lua files
    const completionProvider = vscode.languages.registerCompletionItemProvider(
        { language: 'lua' },
        new SoliDBCompletionProvider(),
        '.', ':' // Trigger on . and :
    );

    // Register hover provider for documentation
    const hoverProvider = vscode.languages.registerHoverProvider(
        { language: 'lua' },
        new SoliDBHoverProvider()
    );

    // Register commands
    const deployCommand = vscode.commands.registerCommand('solidb.deploy', deployScript);
    const replCommand = vscode.commands.registerCommand('solidb.runInRepl', runInRepl);
    const configureCommand = vscode.commands.registerCommand('solidb.configure', configureServer);
    const logsCommand = vscode.commands.registerCommand('solidb.streamLogs', streamLogs);

    context.subscriptions.push(
        completionProvider,
        hoverProvider,
        deployCommand,
        replCommand,
        configureCommand,
        logsCommand
    );
}

// Deactivation function
export function deactivate() {}

// Completion provider for SoliDB APIs
class SoliDBCompletionProvider implements vscode.CompletionItemProvider {
    private completions: Map<string, vscode.CompletionItem[]>;

    constructor() {
        this.completions = new Map();
        this.initializeCompletions();
    }

    private initializeCompletions() {
        // solidb.* completions
        this.completions.set('solidb', [
            this.createCompletion('log', 'Log a message to the _logs collection', 'solidb.log(message)', vscode.CompletionItemKind.Function),
            this.createCompletion('stats', 'Get script execution statistics', 'solidb.stats()', vscode.CompletionItemKind.Function),
            this.createCompletion('now', 'Get current Unix timestamp', 'solidb.now()', vscode.CompletionItemKind.Function),
            this.createCompletion('fetch', 'Make an HTTP request', 'solidb.fetch(url, options)', vscode.CompletionItemKind.Function),
            this.createCompletion('ws', 'WebSocket API (recv, send, close)', 'solidb.ws', vscode.CompletionItemKind.Property),
        ]);

        // print function
        this.completions.set('print', [
            this.createCompletion('print', 'Print values to console output', 'print(...)', vscode.CompletionItemKind.Function),
        ]);

        // db.* completions
        this.completions.set('db', [
            this.createCompletion('collection', 'Get a collection handle', 'db:collection("name")', vscode.CompletionItemKind.Function),
            this.createCompletion('query', 'Execute a SDBQL query', 'db:query("query", bindVars)', vscode.CompletionItemKind.Function),
            this.createCompletion('transaction', 'Execute code in a transaction', 'db:transaction(function(tx) end)', vscode.CompletionItemKind.Function),
            this.createCompletion('enqueue', 'Enqueue a background job', 'db:enqueue("queue", "script", params, { delay_ms = 0 })', vscode.CompletionItemKind.Function),
        ]);

        // response.* completions
        this.completions.set('response', [
            this.createCompletion('json', 'Convert Lua table to JSON string', 'response.json(data)', vscode.CompletionItemKind.Function),
        ]);

        // Collection method completions (after db:collection(""))
        this.completions.set('collection', [
            this.createCompletion('get', 'Get a document by key', ':get("key")', vscode.CompletionItemKind.Method),
            this.createCompletion('insert', 'Insert a document', ':insert(document)', vscode.CompletionItemKind.Method),
            this.createCompletion('update', 'Update a document', ':update("key", document)', vscode.CompletionItemKind.Method),
            this.createCompletion('delete', 'Delete a document', ':delete("key")', vscode.CompletionItemKind.Method),
            this.createCompletion('count', 'Count documents', ':count()', vscode.CompletionItemKind.Method),
        ]);

        // request.* completions
        this.completions.set('request', [
            this.createCompletion('method', 'HTTP method (GET, POST, PUT, DELETE)', 'request.method', vscode.CompletionItemKind.Property),
            this.createCompletion('path', 'Request path', 'request.path', vscode.CompletionItemKind.Property),
            this.createCompletion('params', 'URL path parameters', 'request.params', vscode.CompletionItemKind.Property),
            this.createCompletion('query', 'Query string parameters', 'request.query', vscode.CompletionItemKind.Property),
            this.createCompletion('headers', 'Request headers', 'request.headers', vscode.CompletionItemKind.Property),
            this.createCompletion('body', 'Request body (parsed JSON)', 'request.body', vscode.CompletionItemKind.Property),
            this.createCompletion('is_websocket', 'Whether this is a WebSocket upgrade', 'request.is_websocket', vscode.CompletionItemKind.Property),
        ]);

        // crypto.* completions
        this.completions.set('crypto', [
            this.createCompletion('md5', 'MD5 hash', 'crypto.md5(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('sha256', 'SHA256 hash', 'crypto.sha256(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('sha512', 'SHA512 hash', 'crypto.sha512(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('hmac_sha256', 'HMAC-SHA256 signature', 'crypto.hmac_sha256(key, data)', vscode.CompletionItemKind.Function),
            this.createCompletion('hmac_sha512', 'HMAC-SHA512 signature', 'crypto.hmac_sha512(key, data)', vscode.CompletionItemKind.Function),
            this.createCompletion('base64_encode', 'Base64 encode', 'crypto.base64_encode(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('base64_decode', 'Base64 decode', 'crypto.base64_decode(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('base32_encode', 'Base32 encode', 'crypto.base32_encode(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('base32_decode', 'Base32 decode', 'crypto.base32_decode(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('hex_encode', 'Hex encode binary data', 'crypto.hex_encode(data)', vscode.CompletionItemKind.Function),
            this.createCompletion('hex_decode', 'Hex decode to binary', 'crypto.hex_decode(hex)', vscode.CompletionItemKind.Function),
            this.createCompletion('uuid', 'Generate UUID v4', 'crypto.uuid()', vscode.CompletionItemKind.Function),
            this.createCompletion('uuid_v7', 'Generate time-ordered UUID v7', 'crypto.uuid_v7()', vscode.CompletionItemKind.Function),
            this.createCompletion('random_bytes', 'Generate random bytes', 'crypto.random_bytes(length)', vscode.CompletionItemKind.Function),
            this.createCompletion('curve25519', 'X25519 key exchange', 'crypto.curve25519(secret, public)', vscode.CompletionItemKind.Function),
            this.createCompletion('hash_password', 'Hash password with Argon2', 'crypto.hash_password(password)', vscode.CompletionItemKind.Function),
            this.createCompletion('verify_password', 'Verify password against hash', 'crypto.verify_password(password, hash)', vscode.CompletionItemKind.Function),
            this.createCompletion('jwt_encode', 'Encode JWT token', 'crypto.jwt_encode(claims, secret)', vscode.CompletionItemKind.Function),
            this.createCompletion('jwt_decode', 'Decode JWT token', 'crypto.jwt_decode(token, secret)', vscode.CompletionItemKind.Function),
        ]);

        // time.* completions
        this.completions.set('time', [
            this.createCompletion('now', 'Current time in seconds (float)', 'time.now()', vscode.CompletionItemKind.Function),
            this.createCompletion('now_ms', 'Current time in milliseconds (integer)', 'time.now_ms()', vscode.CompletionItemKind.Function),
            this.createCompletion('iso', 'Current time as ISO string', 'time.iso()', vscode.CompletionItemKind.Function),
            this.createCompletion('sleep', 'Sleep for milliseconds', 'time.sleep(ms)', vscode.CompletionItemKind.Function),
            this.createCompletion('format', 'Format timestamp', 'time.format(timestamp, format)', vscode.CompletionItemKind.Function),
            this.createCompletion('parse', 'Parse ISO string to timestamp', 'time.parse(isoString)', vscode.CompletionItemKind.Function),
            this.createCompletion('add', 'Add time to timestamp', 'time.add(timestamp, value, unit)', vscode.CompletionItemKind.Function),
            this.createCompletion('subtract', 'Subtract time from timestamp', 'time.subtract(timestamp, value, unit)', vscode.CompletionItemKind.Function),
        ]);

        // ws.* completions (WebSocket)
        this.completions.set('ws', [
            this.createCompletion('send', 'Send a WebSocket message', 'ws.send(message)', vscode.CompletionItemKind.Function),
            this.createCompletion('recv', 'Receive a WebSocket message', 'ws.recv()', vscode.CompletionItemKind.Function),
        ]);
    }

    private createCompletion(label: string, documentation: string, detail: string, kind: vscode.CompletionItemKind): vscode.CompletionItem {
        const item = new vscode.CompletionItem(label, kind);
        item.documentation = new vscode.MarkdownString(documentation);
        item.detail = detail;
        return item;
    }

    provideCompletionItems(
        document: vscode.TextDocument,
        position: vscode.Position
    ): vscode.CompletionItem[] | undefined {
        const linePrefix = document.lineAt(position).text.substring(0, position.character);

        // Check what we're completing
        for (const [prefix, items] of this.completions) {
            const regex = new RegExp(`\\b${prefix}[.:]$`);
            if (regex.test(linePrefix)) {
                return items;
            }
        }

        // Check for collection method completions
        if (/db:collection\([^)]+\):$/.test(linePrefix) || /\w+:$/.test(linePrefix)) {
            // Could be a collection variable
            return this.completions.get('collection');
        }

        return undefined;
    }
}

// Hover provider for documentation
class SoliDBHoverProvider implements vscode.HoverProvider {
    private docs: Map<string, string>;

    constructor() {
        this.docs = new Map();
        this.initializeDocs();
    }

    private initializeDocs() {
        this.docs.set('solidb.log', '**solidb.log(message)**\n\nLog a message to the `_logs` collection.\n\n```lua\nsolidb.log("Hello, world!")\nsolidb.log({ user = "alice", action = "login" })\n```');
        this.docs.set('solidb.fetch', '**solidb.fetch(url, options)**\n\nMake an HTTP request.\n\n```lua\nlocal response = solidb.fetch("https://api.example.com/data", {\n  method = "POST",\n  headers = { ["Content-Type"] = "application/json" },\n  body = { key = "value" }\n})\n```');
        this.docs.set('db:collection', '**db:collection(name)**\n\nGet a handle to a collection for CRUD operations.\n\n```lua\nlocal users = db:collection("users")\nlocal doc = users:get("user123")\nusers:insert({ name = "Alice", age = 30 })\n```');
        this.docs.set('db:query', '**db:query(query, bindVars)**\n\nExecute a SDBQL query.\n\n```lua\nlocal results = db:query(\n  "FOR u IN users FILTER u.age > @minAge RETURN u",\n  { minAge = 18 }\n)\n```');
        this.docs.set('crypto.jwt_encode', '**crypto.jwt_encode(claims, secret)**\n\nCreate a JWT token.\n\n```lua\nlocal token = crypto.jwt_encode({\n  sub = "user123",\n  exp = time.now() + 3600\n}, "my-secret-key")\n```');
    }

    provideHover(document: vscode.TextDocument, position: vscode.Position): vscode.Hover | undefined {
        const range = document.getWordRangeAtPosition(position, /[\w.:]+/);
        if (!range) {
            return undefined;
        }

        const word = document.getText(range);

        // Check for exact match or partial match
        for (const [key, doc] of this.docs) {
            if (word.includes(key) || key.includes(word)) {
                return new vscode.Hover(new vscode.MarkdownString(doc));
            }
        }

        return undefined;
    }
}

// Command: Deploy script to SoliDB
async function deployScript() {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
        vscode.window.showErrorMessage('No active editor');
        return;
    }

    const config = vscode.workspace.getConfiguration('solidb');
    const serverUrl = config.get<string>('serverUrl') || 'http://localhost:8080';
    const apiKey = config.get<string>('apiKey') || '';
    const defaultDb = config.get<string>('defaultDatabase') || '_system';

    // Get script details from user
    const database = await vscode.window.showInputBox({
        prompt: 'Database name',
        value: defaultDb
    });
    if (!database) { return; }

    const scriptPath = await vscode.window.showInputBox({
        prompt: 'Script path (e.g., "users/:id" or "hello")',
        placeHolder: 'api/endpoint'
    });
    if (!scriptPath) { return; }

    const scriptName = await vscode.window.showInputBox({
        prompt: 'Script name',
        value: scriptPath.replace(/[/:]/g, '_')
    });
    if (!scriptName) { return; }

    const methods = await vscode.window.showQuickPick(
        ['GET', 'POST', 'PUT', 'DELETE', 'WS'],
        { canPickMany: true, placeHolder: 'Select HTTP methods' }
    );
    if (!methods || methods.length === 0) { return; }

    const code = editor.document.getText();

    try {
        const response = await fetch(`${serverUrl}/_api/database/${database}/scripts`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'X-API-Key': apiKey
            },
            body: JSON.stringify({
                name: scriptName,
                path: scriptPath,
                methods: methods,
                code: code
            })
        });

        if (response.ok) {
            const data = await response.json() as { id: string };
            vscode.window.showInformationMessage(`Script deployed successfully! ID: ${data.id}`);
        } else {
            const error = await response.json() as { error: string };
            vscode.window.showErrorMessage(`Deploy failed: ${error.error}`);
        }
    } catch (error) {
        vscode.window.showErrorMessage(`Deploy failed: ${error}`);
    }
}

// Command: Run selection in REPL
async function runInRepl() {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
        vscode.window.showErrorMessage('No active editor');
        return;
    }

    const config = vscode.workspace.getConfiguration('solidb');
    const serverUrl = config.get<string>('serverUrl') || 'http://localhost:8080';
    const apiKey = config.get<string>('apiKey') || '';
    const defaultDb = config.get<string>('defaultDatabase') || '_system';

    // Get selected text or entire document
    const selection = editor.selection;
    const code = selection.isEmpty
        ? editor.document.getText()
        : editor.document.getText(selection);

    // Create output channel
    const outputChannel = vscode.window.createOutputChannel('SoliDB REPL');
    outputChannel.show();
    outputChannel.appendLine(`Executing in database: ${defaultDb}`);
    outputChannel.appendLine('---');

    try {
        const response = await fetch(`${serverUrl}/_api/database/${defaultDb}/repl`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'X-API-Key': apiKey
            },
            body: JSON.stringify({ code })
        });

        const data = await response.json() as {
            result: unknown;
            output: string[];
            error?: { message: string; line?: number };
            execution_time_ms: number;
        };

        // Display console output
        if (data.output && data.output.length > 0) {
            outputChannel.appendLine('Console:');
            data.output.forEach((line: string) => outputChannel.appendLine(`  > ${line}`));
            outputChannel.appendLine('');
        }

        // Display result or error
        if (data.error) {
            outputChannel.appendLine(`Error${data.error.line ? ` (line ${data.error.line})` : ''}:`);
            outputChannel.appendLine(`  ${data.error.message}`);
        } else {
            outputChannel.appendLine('Result:');
            outputChannel.appendLine(JSON.stringify(data.result, null, 2));
        }

        outputChannel.appendLine('---');
        outputChannel.appendLine(`Executed in ${data.execution_time_ms?.toFixed(2)}ms`);

    } catch (error) {
        outputChannel.appendLine(`Error: ${error}`);
    }
}

// Command: Configure server
async function configureServer() {
    const config = vscode.workspace.getConfiguration('solidb');

    const serverUrl = await vscode.window.showInputBox({
        prompt: 'SoliDB Server URL',
        value: config.get<string>('serverUrl') || 'http://localhost:8080'
    });
    if (serverUrl) {
        await config.update('serverUrl', serverUrl, vscode.ConfigurationTarget.Global);
    }

    const apiKey = await vscode.window.showInputBox({
        prompt: 'API Key (leave empty for none)',
        value: config.get<string>('apiKey') || '',
        password: true
    });
    if (apiKey !== undefined) {
        await config.update('apiKey', apiKey, vscode.ConfigurationTarget.Global);
    }

    const defaultDb = await vscode.window.showInputBox({
        prompt: 'Default database',
        value: config.get<string>('defaultDatabase') || '_system'
    });
    if (defaultDb) {
        await config.update('defaultDatabase', defaultDb, vscode.ConfigurationTarget.Global);
    }

    vscode.window.showInformationMessage('SoliDB configuration updated');
}

// Command: Stream logs (placeholder)
async function streamLogs() {
    vscode.window.showInformationMessage('Log streaming coming soon!');
}
