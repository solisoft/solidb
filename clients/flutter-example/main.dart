import 'package:flutter/material.dart';
import 'package:solidb_flutter/solidb_flutter.dart';

void main() {
  runApp(const SoliDBTodoApp());
}

// SoliDB Configuration
final solidbConfig = SoliDBConfig(
  serverUrl: 'https://your-server.com:6745',
  apiKey: 'your-api-key',
  database: 'mydb',
  collections: ['todos'],
  enableOfflineSync: true,
  syncInterval: const Duration(seconds: 30),
);

class SoliDBTodoApp extends StatelessWidget {
  const SoliDBTodoApp({Key? key}) : super(key: key);

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'SoliDB Todos',
      debugShowCheckedModeBanner: false,
      theme: ThemeData.dark().copyWith(
        scaffoldBackgroundColor: const Color(0xFF0F172A),
        primaryColor: const Color(0xFF3B82F6),
        colorScheme: ColorScheme.dark(
          primary: const Color(0xFF3B82F6),
          secondary: const Color(0xFF10B981),
          surface: const Color(0xFF1E293B),
          background: const Color(0xFF0F172A),
        ),
      ),
      home: const TodoScreen(),
    );
  }
}

class TodoScreen extends StatefulWidget {
  const TodoScreen({Key? key}) : super(key: key);

  @override
  State<TodoScreen> createState() => _TodoScreenState();
}

class _TodoScreenState extends State<TodoScreen> {
  final TextEditingController _controller = TextEditingController();
  late SoliDBClient _client;

  @override
  void initState() {
    super.initState();
    _client = SoliDBClient(config: solidbConfig);
  }

  @override
  void dispose() {
    _controller.dispose();
    _client.dispose();
    super.dispose();
  }

  void _addTodo() async {
    if (_controller.text.trim().isEmpty) return;

    final todo = {
      '_key': DateTime.now().millisecondsSinceEpoch.toString(),
      'text': _controller.text.trim(),
      'completed': false,
      'createdAt': DateTime.now().toIso8601String(),
    };

    try {
      await _client.saveDocument('todos', todo['_key']!, todo);
      _controller.clear();
    } catch (e) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e')),
      );
    }
  }

  void _toggleTodo(String key, bool completed) async {
    try {
      final doc = await _client.getDocument('todos', key);
      if (doc != null) {
        await _client.saveDocument('todos', key, {
          ...doc,
          'completed': completed,
          'updatedAt': DateTime.now().toIso8601String(),
        });
      }
    } catch (e) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e')),
      );
    }
  }

  void _deleteTodo(String key) async {
    try {
      await _client.deleteDocument('todos', key);
    } catch (e) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SoliDB Todos'),
        backgroundColor: const Color(0xFF1E293B),
        elevation: 0,
        actions: [
          // Online/Offline indicator
          StreamBuilder<SyncStatus>(
            stream: _client.syncStatus,
            builder: (context, snapshot) {
              final isOnline = snapshot.data?.isOnline ?? false;
              final pendingCount = snapshot.data?.pendingChanges ?? 0;
              
              return Row(
                children: [
                  Container(
                    width: 8,
                    height: 8,
                    margin: const EdgeInsets.only(right: 8),
                    decoration: BoxDecoration(
                      color: isOnline ? Colors.green : Colors.red,
                      shape: BoxShape.circle,
                    ),
                  ),
                  Text(
                    isOnline ? 'Online' : 'Offline',
                    style: const TextStyle(fontSize: 12),
                  ),
                  if (pendingCount > 0)
                    Padding(
                      padding: const EdgeInsets.only(left: 8, right: 16),
                      child: Text(
                        '($pendingCount pending)',
                        style: const TextStyle(
                          fontSize: 12,
                          color: Colors.orange,
                        ),
                      ),
                    ),
                ],
              );
            },
          ),
        ],
      ),
      body: Column(
        children: [
          // Add Todo Input
          Container(
            padding: const EdgeInsets.all(16),
            color: const Color(0xFF1E293B),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _controller,
                    decoration: InputDecoration(
                      hintText: 'What needs to be done?',
                      hintStyle: TextStyle(color: Colors.grey.shade400),
                      filled: true,
                      fillColor: const Color(0xFF334155),
                      border: OutlineInputBorder(
                        borderRadius: BorderRadius.circular(8),
                        borderSide: BorderSide.none,
                      ),
                      contentPadding: const EdgeInsets.symmetric(
                        horizontal: 16,
                        vertical: 12,
                      ),
                    ),
                    style: const TextStyle(color: Colors.white),
                    onSubmitted: (_) => _addTodo(),
                  ),
                ),
                const SizedBox(width: 12),
                ElevatedButton(
                  onPressed: _addTodo,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: const Color(0xFF3B82F6),
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 24,
                      vertical: 12,
                    ),
                    shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(8),
                    ),
                  ),
                  child: const Text('Add'),
                ),
              ],
            ),
          ),
          
          // Todo List with real-time sync
          Expanded(
            child: SoliDBStreamBuilder<List<Map<String, dynamic>>>(
              stream: _client.watchCollection('todos'),
              builder: (context, snapshot) {
                if (snapshot.connectionState == ConnectionState.waiting) {
                  return const Center(
                    child: CircularProgressIndicator(
                      valueColor: AlwaysStoppedAnimation<Color>(
                        Color(0xFF3B82F6),
                      ),
                    ),
                  );
                }

                if (snapshot.hasError) {
                  return Center(
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Text(
                          'Error: ${snapshot.error}',
                          style: const TextStyle(color: Colors.red),
                        ),
                        const SizedBox(height: 16),
                        ElevatedButton(
                          onPressed: () => _client.syncNow(),
                          child: const Text('Retry'),
                        ),
                      ],
                    ),
                  );
                }

                final todos = snapshot.data ?? [];

                if (todos.isEmpty) {
                  return Center(
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Icon(
                          Icons.inbox,
                          size: 64,
                          color: Colors.grey.shade600,
                        ),
                        const SizedBox(height: 16),
                        Text(
                          'No todos yet.\nAdd one above!',
                          textAlign: TextAlign.center,
                          style: TextStyle(
                            color: Colors.grey.shade400,
                            fontSize: 16,
                          ),
                        ),
                      ],
                    ),
                  );
                }

                return ListView.builder(
                  padding: const EdgeInsets.all(16),
                  itemCount: todos.length,
                  itemBuilder: (context, index) {
                    final todo = todos[index];
                    final key = todo['_key'] as String;
                    final text = todo['text'] as String;
                    final completed = todo['completed'] as bool? ?? false;

                    return Card(
                      color: const Color(0xFF1E293B),
                      margin: const EdgeInsets.only(bottom: 12),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(12),
                      ),
                      child: ListTile(
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 16,
                          vertical: 8,
                        ),
                        leading: Checkbox(
                          value: completed,
                          onChanged: (value) => _toggleTodo(key, value!),
                          activeColor: const Color(0xFF3B82F6),
                        ),
                        title: Text(
                          text,
                          style: TextStyle(
                            color: completed ? Colors.grey : Colors.white,
                            decoration: completed
                                ? TextDecoration.lineThrough
                                : null,
                          ),
                        ),
                        trailing: IconButton(
                          icon: const Icon(Icons.delete, color: Colors.red),
                          onPressed: () => _deleteTodo(key),
                        ),
                      ),
                    );
                  },
                );
              },
            ),
          ),
          
          // Sync button at bottom
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: const Color(0xFF1E293B),
              border: Border(
                top: BorderSide(color: Colors.grey.shade800),
              ),
            ),
            child: Row(
              children: [
                Expanded(
                  child: StreamBuilder<SyncStatus>(
                    stream: _client.syncStatus,
                    builder: (context, snapshot) {
                      final lastSync = snapshot.data?.lastSyncTime;
                      return Text(
                        lastSync != null
                            ? 'Last sync: ${_formatTime(lastSync)}'
                            : 'Never synced',
                        style: TextStyle(
                          color: Colors.grey.shade400,
                          fontSize: 12,
                        ),
                      );
                    },
                  ),
                ),
                ElevatedButton.icon(
                  onPressed: () => _client.syncNow(),
                  icon: const Icon(Icons.sync),
                  label: const Text('Sync Now'),
                  style: ElevatedButton.styleFrom(
                    backgroundColor: const Color(0xFF3B82F6),
                    foregroundColor: Colors.white,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  String _formatTime(DateTime time) {
    final now = DateTime.now();
    final diff = now.difference(time);

    if (diff.inSeconds < 60) {
      return 'Just now';
    } else if (diff.inMinutes < 60) {
      return '${diff.inMinutes}m ago';
    } else if (diff.inHours < 24) {
      return '${diff.inHours}h ago';
    } else {
      return '${diff.inDays}d ago';
    }
  }
}

// Extension for the SoliDB client to provide watchCollection
extension SoliDBClientExtension on SoliDBClient {
  Stream<List<Map<String, dynamic>>> watchCollection(String collection) {
    // This would be implemented in the actual SDK
    // For now, returning a placeholder stream
    return Stream.periodic(
      const Duration(seconds: 1),
      (_) => [],
    );
  }
}
