package com.solidb.example

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.solidb.client.*
import com.solidb.example.ui.theme.SoliDBExampleTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private lateinit var syncManager: SyncManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Initialize SoliDB Sync Manager
        initSyncManager()

        setContent {
            SoliDBExampleTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    TodoApp(syncManager = syncManager)
                }
            }
        }
    }

    private fun initSyncManager() {
        val config = SyncConfig(
            deviceId = Utils.generateDeviceId(),
            serverUrl = "https://your-server.com:6745",
            apiKey = "your-api-key",
            collections = listOf("todos"),
            syncIntervalSecs = 30,
            maxRetries = 5,
            autoSync = true
        )

        syncManager = SyncManager(config)
        syncManager.start()
    }

    override fun onDestroy() {
        super.onDestroy()
        syncManager.stop()
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TodoApp(syncManager: SyncManager) {
    var todos by remember { mutableStateOf(listOf<Todo>()) }
    var newTodoText by remember { mutableStateOf("") }
    var isOnline by remember { mutableStateOf(syncManager.isOnline()) }
    var pendingCount by remember { mutableStateOf(0L) }
    val scope = rememberCoroutineScope()

    // Load todos on startup
    LaunchedEffect(Unit) {
        loadTodos(syncManager) { todos = it }
        updateStats(syncManager) { count, online ->
            pendingCount = count
            isOnline = online
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("SoliDB Offline Todos") },
                actions = {
                    // Sync button
                    IconButton(onClick = {
                        scope.launch {
                            syncManager.syncNow()
                            loadTodos(syncManager) { todos = it }
                        }
                    }) {
                        Icon(
                            imageVector = if (isOnline) 
                                androidx.compose.material.icons.Icons.Default.CloudDone 
                            else 
                                androidx.compose.material.icons.Icons.Default.CloudOff,
                            contentDescription = "Sync",
                            tint = if (isOnline) MaterialTheme.colorScheme.primary 
                                   else MaterialTheme.colorScheme.error
                        )
                    }
                    
                    // Pending count badge
                    if (pendingCount > 0) {
                        Badge {
                            Text("$pendingCount")
                        }
                    }
                }
            )
        }
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
                .padding(16.dp)
        ) {
            // Add new todo
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = newTodoText,
                    onValueChange = { newTodoText = it },
                    label = { Text("New todo") },
                    modifier = Modifier.weight(1f)
                )
                
                Spacer(modifier = Modifier.width(8.dp))
                
                Button(
                    onClick = {
                        if (newTodoText.isNotBlank()) {
                            scope.launch {
                                addTodo(syncManager, newTodoText) { success ->
                                    if (success) {
                                        newTodoText = ""
                                        loadTodos(syncManager) { todos = it }
                                        updateStats(syncManager) { count, online ->
                                            pendingCount = count
                                            isOnline = online
                                        }
                                    }
                                }
                            }
                        }
                    }
                ) {
                    Text("Add")
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // Status indicator
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = if (isOnline) 
                        MaterialTheme.colorScheme.primaryContainer 
                    else 
                        MaterialTheme.colorScheme.errorContainer
                )
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = if (isOnline) "🟢 Online" else "🔴 Offline",
                        style = MaterialTheme.typography.bodyLarge
                    )
                    Text(
                        text = "$pendingCount pending",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // Todo list
            LazyColumn {
                items(todos) { todo ->
                    TodoItem(
                        todo = todo,
                        onToggle = {
                            scope.launch {
                                toggleTodo(syncManager, todo) {
                                    loadTodos(syncManager) { todos = it }
                                }
                            }
                        },
                        onDelete = {
                            scope.launch {
                                deleteTodo(syncManager, todo.id) {
                                    loadTodos(syncManager) { todos = it }
                                    updateStats(syncManager) { count, online ->
                                        pendingCount = count
                                        isOnline = online
                                    }
                                }
                            }
                        }
                    )
                    Divider()
                }
            }
        }
    }
}

@Composable
fun TodoItem(
    todo: Todo,
    onToggle: () -> Unit,
    onDelete: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Checkbox(
            checked = todo.completed,
            onCheckedChange = { onToggle() }
        )
        
        Text(
            text = todo.text,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyLarge,
            textDecoration = if (todo.completed) 
                androidx.compose.ui.text.style.TextDecoration.LineThrough 
            else 
                androidx.compose.ui.text.style.TextDecoration.None
        )
        
        IconButton(onClick = onDelete) {
            Icon(
                imageVector = androidx.compose.material.icons.Icons.Default.Delete,
                contentDescription = "Delete",
                tint = MaterialTheme.colorScheme.error
            )
        }
    }
}

// Data class
data class Todo(
    val id: String,
    val text: String,
    val completed: Boolean
)

// Helper functions
private fun loadTodos(syncManager: SyncManager, onResult: (List<Todo>) -> Unit) {
    try {
        val documents = syncManager.queryDocuments("todos")
        val todos = documents.map { doc ->
            val json = org.json.JSONObject(doc.data)
            Todo(
                id = doc.id,
                text = json.optString("text", ""),
                completed = json.optBoolean("completed", false)
            )
        }
        onResult(todos)
    } catch (e: Exception) {
        e.printStackTrace()
        onResult(emptyList())
    }
}

private fun addTodo(syncManager: SyncManager, text: String, onResult: (Boolean) -> Unit) {
    try {
        val id = System.currentTimeMillis().toString()
        val json = org.json.JSONObject().apply {
            put("text", text)
            put("completed", false)
            put("created_at", System.currentTimeMillis())
        }
        
        syncManager.saveDocument("todos", id, json.toString())
        onResult(true)
    } catch (e: Exception) {
        e.printStackTrace()
        onResult(false)
    }
}

private fun toggleTodo(syncManager: SyncManager, todo: Todo, onResult: () -> Unit) {
    try {
        val json = org.json.JSONObject().apply {
            put("text", todo.text)
            put("completed", !todo.completed)
            put("updated_at", System.currentTimeMillis())
        }
        
        syncManager.saveDocument("todos", todo.id, json.toString())
        onResult()
    } catch (e: Exception) {
        e.printStackTrace()
    }
}

private fun deleteTodo(syncManager: SyncManager, id: String, onResult: () -> Unit) {
    try {
        syncManager.deleteDocument("todos", id)
        onResult()
    } catch (e: Exception) {
        e.printStackTrace()
    }
}

private fun updateStats(
    syncManager: SyncManager,
    onResult: (Long, Boolean) -> Unit
) {
    try {
        val pending = syncManager.pendingCount
        val online = syncManager.isOnline()
        onResult(pending, online)
    } catch (e: Exception) {
        e.printStackTrace()
        onResult(0, false)
    }
}
