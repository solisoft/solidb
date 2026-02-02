import React, { useState, useCallback } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  FlatList,
  StyleSheet,
  SafeAreaView,
  StatusBar,
  ActivityIndicator,
} from 'react-native';
import { useSync, useDocument, useCollection } from '@solidb/react-native';

// Configure SoliDB
const solidbConfig = {
  serverUrl: 'https://your-server.com:6745',
  apiKey: 'your-api-key',
  database: 'mydb',
  collections: ['todos'],
  enableOfflineSync: true,
  syncInterval: 30000, // 30 seconds
};

// Todo Item Component
const TodoItem = ({ todo, onToggle, onDelete }) => {
  return (
    <View style={styles.todoItem}>
      <TouchableOpacity
        style={styles.checkbox}
        onPress={() => onToggle(todo._key, !todo.completed)}
      >
        <Text style={styles.checkboxText}>
          {todo.completed ? '✓' : '○'}
        </Text>
      </TouchableOpacity>
      
      <Text
        style={[
          styles.todoText,
          todo.completed && styles.todoTextCompleted,
        ]}
      >
        {todo.text}
      </Text>
      
      <TouchableOpacity
        style={styles.deleteButton}
        onPress={() => onDelete(todo._key)}
      >
        <Text style={styles.deleteButtonText}>🗑</Text>
      </TouchableOpacity>
    </View>
  );
};

// Main App Component
export default function App() {
  const [newTodo, setNewTodo] = useState('');
  
  // Use the sync hook for the todos collection
  const {
    data: todos,
    loading,
    error,
    isOnline,
    pendingChanges,
    refresh,
    saveDocument,
    deleteDocument,
  } = useCollection('todos', solidbConfig);

  // Add new todo
  const handleAddTodo = useCallback(async () => {
    if (!newTodo.trim()) return;
    
    const todo = {
      _key: Date.now().toString(),
      text: newTodo.trim(),
      completed: false,
      createdAt: new Date().toISOString(),
    };
    
    try {
      await saveDocument(todo._key, todo);
      setNewTodo('');
    } catch (err) {
      console.error('Failed to add todo:', err);
    }
  }, [newTodo, saveDocument]);

  // Toggle todo completion
  const handleToggleTodo = useCallback(async (key, completed) => {
    const todo = todos.find(t => t._key === key);
    if (!todo) return;
    
    try {
      await saveDocument(key, {
        ...todo,
        completed,
        updatedAt: new Date().toISOString(),
      });
    } catch (err) {
      console.error('Failed to toggle todo:', err);
    }
  }, [todos, saveDocument]);

  // Delete todo
  const handleDeleteTodo = useCallback(async (key) => {
    try {
      await deleteDocument(key);
    } catch (err) {
      console.error('Failed to delete todo:', err);
    }
  }, [deleteDocument]);

  // Render
  return (
    <SafeAreaView style={styles.container}>
      <StatusBar barStyle="light-content" />
      
      {/* Header */}
      <View style={styles.header}>
        <Text style={styles.title}>SoliDB Todos</Text>
        <View style={styles.statusContainer}>
          <View style={[
            styles.statusIndicator,
            { backgroundColor: isOnline ? '#4ade80' : '#ef4444' }
          ]} />
          <Text style={styles.statusText}>
            {isOnline ? 'Online' : 'Offline'}
          </Text>
          {pendingChanges > 0 && (
            <Text style={styles.pendingText}>
              ({pendingChanges} pending)
            </Text>
          )}
        </View>
      </View>

      {/* Add Todo Input */}
      <View style={styles.inputContainer}>
        <TextInput
          style={styles.input}
          value={newTodo}
          onChangeText={setNewTodo}
          placeholder="What needs to be done?"
          placeholderTextColor="#9ca3af"
          onSubmitEditing={handleAddTodo}
          returnKeyType="done"
        />
        <TouchableOpacity
          style={[styles.addButton, !newTodo.trim() && styles.addButtonDisabled]}
          onPress={handleAddTodo}
          disabled={!newTodo.trim()}
        >
          <Text style={styles.addButtonText}>Add</Text>
        </TouchableOpacity>
      </View>

      {/* Todo List */}
      {loading ? (
        <ActivityIndicator size="large" color="#3b82f6" style={styles.loader} />
      ) : error ? (
        <View style={styles.errorContainer}>
          <Text style={styles.errorText}>Error: {error.message}</Text>
          <TouchableOpacity onPress={refresh} style={styles.retryButton}>
            <Text style={styles.retryButtonText}>Retry</Text>
          </TouchableOpacity>
        </View>
      ) : (
        <FlatList
          data={todos}
          keyExtractor={(item) => item._key}
          renderItem={({ item }) => (
            <TodoItem
              todo={item}
              onToggle={handleToggleTodo}
              onDelete={handleDeleteTodo}
            />
          )}
          contentContainerStyle={styles.list}
          ListEmptyComponent={
            <View style={styles.emptyContainer}>
              <Text style={styles.emptyText}>
                No todos yet. Add one above!
              </Text>
            </View>
          }
        />
      )}

      {/* Sync Button */}
      <TouchableOpacity
        style={styles.syncButton}
        onPress={refresh}
        disabled={loading}
      >
        <Text style={styles.syncButtonText}>
          {loading ? 'Syncing...' : 'Sync Now'}
        </Text>
      </TouchableOpacity>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#0f172a',
  },
  header: {
    padding: 20,
    backgroundColor: '#1e293b',
    borderBottomWidth: 1,
    borderBottomColor: '#334155',
  },
  title: {
    fontSize: 28,
    fontWeight: 'bold',
    color: '#f8fafc',
    marginBottom: 8,
  },
  statusContainer: {
    flexDirection: 'row',
    alignItems: 'center',
  },
  statusIndicator: {
    width: 8,
    height: 8,
    borderRadius: 4,
    marginRight: 8,
  },
  statusText: {
    color: '#94a3b8',
    fontSize: 14,
  },
  pendingText: {
    color: '#f59e0b',
    fontSize: 14,
    marginLeft: 8,
  },
  inputContainer: {
    flexDirection: 'row',
    padding: 16,
    backgroundColor: '#1e293b',
    borderBottomWidth: 1,
    borderBottomColor: '#334155',
  },
  input: {
    flex: 1,
    backgroundColor: '#334155',
    borderRadius: 8,
    padding: 12,
    color: '#f8fafc',
    fontSize: 16,
    marginRight: 12,
  },
  addButton: {
    backgroundColor: '#3b82f6',
    borderRadius: 8,
    paddingHorizontal: 20,
    paddingVertical: 12,
    justifyContent: 'center',
  },
  addButtonDisabled: {
    backgroundColor: '#475569',
  },
  addButtonText: {
    color: '#ffffff',
    fontWeight: '600',
    fontSize: 16,
  },
  list: {
    padding: 16,
  },
  todoItem: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#1e293b',
    borderRadius: 12,
    padding: 16,
    marginBottom: 12,
  },
  checkbox: {
    marginRight: 12,
  },
  checkboxText: {
    fontSize: 24,
    color: '#3b82f6',
  },
  todoText: {
    flex: 1,
    fontSize: 16,
    color: '#f8fafc',
  },
  todoTextCompleted: {
    textDecorationLine: 'line-through',
    color: '#64748b',
  },
  deleteButton: {
    padding: 8,
  },
  deleteButtonText: {
    fontSize: 20,
  },
  loader: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
  },
  errorContainer: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 20,
  },
  errorText: {
    color: '#ef4444',
    fontSize: 16,
    marginBottom: 16,
    textAlign: 'center',
  },
  retryButton: {
    backgroundColor: '#3b82f6',
    borderRadius: 8,
    paddingHorizontal: 24,
    paddingVertical: 12,
  },
  retryButtonText: {
    color: '#ffffff',
    fontWeight: '600',
    fontSize: 16,
  },
  emptyContainer: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 40,
  },
  emptyText: {
    color: '#64748b',
    fontSize: 16,
    textAlign: 'center',
  },
  syncButton: {
    backgroundColor: '#1e293b',
    borderTopWidth: 1,
    borderTopColor: '#334155',
    padding: 16,
    alignItems: 'center',
  },
  syncButtonText: {
    color: '#3b82f6',
    fontWeight: '600',
    fontSize: 16,
  },
});
