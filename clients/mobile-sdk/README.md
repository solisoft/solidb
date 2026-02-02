# SoliDB Mobile SDK

Native iOS and Android SDKs for SoliDB with offline-first synchronization. Also includes React Native and Flutter wrappers for cross-platform development.

## Overview

The SoliDB Mobile SDK provides native bindings for iOS (Swift) and Android (Kotlin) using UniFFI to generate FFI bindings from the Rust core. Additionally, we provide React Native and Flutter SDKs for cross-platform development.

### SDK Options

| SDK | Language | Best For | Installation |
|-----|----------|----------|--------------|
| **iOS** | Swift | Native iOS apps | SPM / CocoaPods |
| **Android** | Kotlin | Native Android apps | Gradle |
| **React Native** | TypeScript | Cross-platform JS apps | npm |
| **Flutter** | Dart | Cross-platform apps | pub.dev |

### Common Features

All SDKs provide:
- ✅ **Offline-First** - Built-in SQLite local storage
- ✅ **Auto Sync** - Automatic synchronization when online
- ✅ **Conflict Resolution** - Multiple strategies (LWW, CRDT, Manual)
- ✅ **Delta Sync** - Bandwidth-efficient updates
- ✅ **Type Safety** - Native types in each language

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Your Mobile App                          │
│     (Swift / Kotlin / React Native / Flutter)               │
└──────────────────────┬──────────────────────────────────────┘
                       │ FFI / Bridge / Dart FFI
┌──────────────────────▼──────────────────────────────────────┐
│                    Rust Core (solidb_client)                 │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                    SyncManager                          │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │ │
│  │  │  HTTP Client │  │  LocalStore  │  │ Sync Engine  │  │ │
│  │  │  (reqwest)   │  │  (SQLite)    │  │  (tokio)     │  │ │
│  │  └──────────────┘  └──────────────┘  └──────────────┘  │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### Building the SDK

1. **Install prerequisites:**
   ```bash
   # Rust toolchain
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   
   # UniFFI
   cargo install uniffi-bindgen
   
   # For iOS
   rustup target add aarch64-apple-ios x86_64-apple-ios
   
   # For Android
   rustup target add aarch64-linux-android armv7-linux-androideabi
   ```

2. **Build the bindings:**
   ```bash
   cd clients/rust-client
   
   # Generate Kotlin bindings
   cargo build --release --features mobile
   uniffi-bindgen generate src/solidb_client.udl --language kotlin --out-dir ../../mobile-bindings/kotlin
   
   # Generate Swift bindings
   uniffi-bindgen generate src/solidb_client.udl --language swift --out-dir ../../mobile-bindings/swift
   ```

3. **Build native libraries:**
   ```bash
   # iOS library
   cargo build --release --target aarch64-apple-ios
   
   # Android library
   cargo build --release --target aarch64-linux-android
   ```

### iOS Integration

1. **Add the framework to your project:**
   - Drag `SoliDBClient.xcframework` into your Xcode project
   - Add to Frameworks, Libraries, and Embedded Content

2. **Use in your Swift code:**
   ```swift
   import SoliDBClient
   
   let config = SyncConfig(
       deviceId: Utils.generateDeviceId(),
       serverUrl: "https://your-server.com:6745",
       apiKey: "your-api-key",
       collections: ["todos"],
       syncIntervalSecs: 30,
       maxRetries: 5,
       autoSync: true
   )
   
   let syncManager = try SyncManager(config: config)
   syncManager.start()
   
   // Save data (works offline!)
   let todo = Todo(id: "1", text: "Buy milk", completed: false)
   let json = try JSONEncoder().encode(todo)
   try syncManager.saveDocument(
       collection: "todos", 
       key: todo.id, 
       data: String(data: json, encoding: .utf8)!
   )
   
   // Query data
   let documents = try syncManager.queryDocuments(collection: "todos")
   ```

See full iOS example in `clients/ios-example/`

### Android Integration

1. **Add the AAR to your project:**
   ```kotlin
   // build.gradle.kts
   dependencies {
       implementation(files("libs/solidb_client.aar"))
   }
   ```

2. **Use in your Kotlin code:**
   ```kotlin
   import com.solidb.client.*
   
   val config = SyncConfig(
       deviceId = Utils.generateDeviceId(),
       serverUrl = "https://your-server.com:6745",
       apiKey = "your-api-key",
       collections = listOf("todos"),
       syncIntervalSecs = 30,
       maxRetries = 5,
       autoSync = true
   )
   
   val syncManager = SyncManager(config)
   syncManager.start()
   
   // Save data (works offline!)
   val todo = JSONObject().apply {
       put("text", "Buy milk")
       put("completed", false)
   }
   syncManager.saveDocument("todos", "1", todo.toString())
   
   // Query data
   val documents = syncManager.queryDocuments("todos")
   ```

See full Android example in `clients/android-example/`

### React Native Integration

1. **Install via npm/yarn:**
   ```bash
   npm install @solidb/react-native
   # or
   yarn add @solidb/react-native
   ```

2. **Expo vs Bare Workflow:**

   **Expo (Managed Workflow):**
   ```bash
   npx expo install @solidb/react-native
   ```
   - Uses Expo's native module system
   - No ejecting required
   - Limited access to low-level sync controls

   **Bare Workflow (React Native CLI):**
   ```bash
   npm install @solidb/react-native
   cd ios && pod install
   ```
   - Full access to all SDK features
   - Manual native configuration required

3. **Basic Usage with Hooks:**

   ```typescript
   import { SoliDBProvider, useSync, useDocument } from '@solidb/react-native';
   
   // Wrap your app with the provider
   function App() {
     return (
       <SoliDBProvider
         config={{
           deviceId: 'unique-device-id',
           serverUrl: 'https://your-server.com:6745',
           apiKey: 'your-api-key',
           collections: ['todos'],
           syncIntervalSecs: 30,
           autoSync: true,
         }}
       >
         <TodoApp />
       </SoliDBProvider>
     );
   }
   
   // Use the sync hook
   function TodoApp() {
     const { isOnline, syncNow, pendingCount } = useSync();
     
     return (
       <View>
         <Text>Status: {isOnline ? 'Online' : 'Offline'}</Text>
         <Text>Pending: {pendingCount} changes</Text>
         <Button title="Sync Now" onPress={syncNow} />
       </View>
     );
   }
   
   // Use the document hook
   function TodoItem({ todoId }: { todoId: string }) {
     const { document, save, remove } = useDocument('todos', todoId);
     
     return (
       <View>
         <Text>{document?.text}</Text>
         <Switch
           value={document?.completed}
           onValueChange={(completed) => save({ ...document, completed })}
         />
       </View>
     );
   }
   ```

4. **Complete Example with Offline Sync:**

   ```typescript
   import React, { useState } from 'react';
   import { View, Text, TextInput, Button, FlatList } from 'react-native';
   import { useSync, useCollection, useDocument } from '@solidb/react-native';
   
   interface Todo {
     id: string;
     text: string;
     completed: boolean;
     createdAt: number;
   }
   
   export default function TodoApp() {
     const { isOnline, syncNow, pendingCount } = useSync();
     const { documents: todos, add, refresh } = useCollection<Todo>('todos');
     const [newTodo, setNewTodo] = useState('');
     
     const handleAddTodo = () => {
       if (!newTodo.trim()) return;
       
       add({
         id: Date.now().toString(),
         text: newTodo,
         completed: false,
         createdAt: Date.now(),
       });
       
       setNewTodo('');
     };
     
     return (
       <View style={{ flex: 1, padding: 20 }}>
         <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
           <Text>{isOnline ? '🟢 Online' : '🔴 Offline'}</Text>
           <Text>Pending: {pendingCount}</Text>
         </View>
         
         <View style={{ flexDirection: 'row', marginVertical: 10 }}>
           <TextInput
             value={newTodo}
             onChangeText={setNewTodo}
             placeholder="Add a todo..."
             style={{ flex: 1, borderWidth: 1, padding: 10 }}
           />
           <Button title="Add" onPress={handleAddTodo} />
         </View>
         
         <FlatList
           data={todos}
           keyExtractor={(item) => item.id}
           renderItem={({ item }) => (
             <TodoItem todo={item} />
           )}
           onRefresh={refresh}
           refreshing={false}
         />
       </View>
     );
   }
   
   function TodoItem({ todo }: { todo: Todo }) {
     const { document, save, remove } = useDocument<Todo>('todos', todo.id);
     
     return (
       <View style={{ flexDirection: 'row', alignItems: 'center', padding: 10 }}>
         <Switch
           value={document?.completed ?? todo.completed}
           onValueChange={(completed) =>
             save({ ...todo, completed })
           }
         />
         <Text style={{ flex: 1, marginLeft: 10 }}>
           {document?.text ?? todo.text}
         </Text>
         <Button title="Delete" onPress={remove} />
       </View>
     );
   }
   ```

5. **iOS/Android Platform Setup:**

   **iOS Configuration:**
   ```bash
   cd ios && pod install
   ```
   - Add to `ios/Podfile`:
   ```ruby
   pod 'SoliDBClient', :path => '../node_modules/@solidb/react-native/ios'
   ```
   - Enable Keychain Sharing in Xcode (for secure token storage)
   - Add Background Modes: Background fetch, Background processing

   **Android Configuration:**
   ```gradle
   // android/settings.gradle
   include ':solidb_react_native'
   project(':solidb_react_native').projectDir = new File(rootProject.projectDir, '../node_modules/@solidb/react-native/android')
   ```
   ```gradle
   // android/app/build.gradle
   dependencies {
       implementation project(':solidb_react_native')
   }
   ```
   - Add to `AndroidManifest.xml`:
   ```xml
   <uses-permission android:name="android.permission.INTERNET" />
   <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
   ```

See full React Native example in `clients/react-native-example/`

### Flutter Integration

1. **Install via pub.dev:**

   Add to `pubspec.yaml`:
   ```yaml
   dependencies:
     solidb_flutter: ^1.0.0
   ```
   ```bash
   flutter pub get
   ```

2. **Dart/Flutter Widget-Based API:**

   ```dart
   import 'package:solidb_flutter/solidb_flutter.dart';
   
   class MyApp extends StatelessWidget {
     @override
     Widget build(BuildContext context) {
       return SoliDBProvider(
         config: SyncConfig(
           deviceId: 'unique-device-id',
           serverUrl: 'https://your-server.com:6745',
           apiKey: 'your-api-key',
           collections: ['todos'],
           syncIntervalSecs: 30,
           autoSync: true,
         ),
         child: MaterialApp(
           home: TodoScreen(),
         ),
       );
     }
   }
   ```

3. **Stream-Based Reactive Sync:**

   ```dart
   import 'package:solidb_flutter/solidb_flutter.dart';
   
   class TodoScreen extends StatelessWidget {
     @override
     Widget build(BuildContext context) {
       return Scaffold(
         appBar: AppBar(
           title: StreamBuilder<bool>(
             stream: context.solidb.onlineStatus,
             builder: (context, snapshot) {
               final isOnline = snapshot.data ?? false;
               return Text(isOnline ? '🟢 Online' : '🔴 Offline');
             },
           ),
         ),
         body: StreamBuilder<List<Document>>(
           stream: context.solidb.watchCollection('todos'),
           builder: (context, snapshot) {
             if (snapshot.hasError) {
               return Center(child: Text('Error: ${snapshot.error}'));
             }
             
             if (!snapshot.hasData) {
               return Center(child: CircularProgressIndicator());
             }
             
             final todos = snapshot.data!;
             
             return ListView.builder(
               itemCount: todos.length,
               itemBuilder: (context, index) {
                 final todo = todos[index];
                 return TodoListItem(todo: todo);
               },
             );
           },
         ),
       );
     }
   }
   
   class TodoListItem extends StatelessWidget {
     final Document todo;
     
     TodoListItem({required this.todo});
     
     @override
     Widget build(BuildContext context) {
       return StreamBuilder<Document?>(
         stream: context.solidb.watchDocument('todos', todo.id),
         builder: (context, snapshot) {
           final data = snapshot.data?.data ?? todo.data;
           
           return ListTile(
             leading: Checkbox(
               value: data['completed'] ?? false,
               onChanged: (value) {
                 context.solidb.saveDocument(
                   'todos',
                   todo.id,
                   { ...data, 'completed': value },
                 );
               },
             ),
             title: Text(data['text'] ?? ''),
             trailing: IconButton(
               icon: Icon(Icons.delete),
               onPressed: () {
                 context.solidb.deleteDocument('todos', todo.id);
               },
             ),
           );
         },
       );
     }
   }
   ```

4. **Complete Example with Offline Support:**

   ```dart
   import 'package:flutter/material.dart';
   import 'package:solidb_flutter/solidb_flutter.dart';
   
   void main() {
     runApp(TodoApp());
   }
   
   class TodoApp extends StatelessWidget {
     @override
     Widget build(BuildContext context) {
       return SoliDBProvider(
         config: SyncConfig(
           deviceId: 'flutter-device-${DateTime.now().millisecondsSinceEpoch}',
           serverUrl: 'https://your-server.com:6745',
           apiKey: 'your-api-key',
           collections: ['todos'],
           syncIntervalSecs: 30,
           autoSync: true,
           enableEncryption: true,
         ),
         child: MaterialApp(
           title: 'SoliDB Todos',
           home: TodoScreen(),
         ),
       );
     }
   }
   
   class TodoScreen extends StatefulWidget {
     @override
     _TodoScreenState createState() => _TodoScreenState();
   }
   
   class _TodoScreenState extends State<TodoScreen> {
     final _textController = TextEditingController();
     late final SoliDBClient _client;
     
     @override
     void didChangeDependencies() {
       super.didChangeDependencies();
       _client = context.solidb;
     }
     
     void _addTodo() {
       if (_textController.text.isEmpty) return;
       
       final todo = {
         'id': DateTime.now().millisecondsSinceEpoch.toString(),
         'text': _textController.text,
         'completed': false,
         'createdAt': DateTime.now().toIso8601String(),
       };
       
       _client.saveDocument('todos', todo['id']!, todo);
       _textController.clear();
     }
     
     @override
     Widget build(BuildContext context) {
       return Scaffold(
         appBar: AppBar(
           title: Text('Offline-First Todos'),
           actions: [
             StreamBuilder<int>(
               stream: _client.pendingChangesCount,
               builder: (context, snapshot) {
                 final count = snapshot.data ?? 0;
                 if (count == 0) return SizedBox.shrink();
                 return Badge(
                   label: Text('$count'),
                   child: IconButton(
                     icon: Icon(Icons.sync),
                     onPressed: () => _client.syncNow(),
                   ),
                 );
               },
             ),
             StreamBuilder<bool>(
               stream: _client.onlineStatus,
               builder: (context, snapshot) {
                 final isOnline = snapshot.data ?? false;
                 return Padding(
                   padding: EdgeInsets.all(8.0),
                   child: Icon(
                     isOnline ? Icons.cloud_done : Icons.cloud_off,
                     color: isOnline ? Colors.green : Colors.grey,
                   ),
                 );
               },
             ),
           ],
         ),
         body: Column(
           children: [
             Padding(
               padding: EdgeInsets.all(16.0),
               child: Row(
                 children: [
                   Expanded(
                     child: TextField(
                       controller: _textController,
                       decoration: InputDecoration(
                         hintText: 'Add a new todo...',
                         border: OutlineInputBorder(),
                       ),
                       onSubmitted: (_) => _addTodo(),
                     ),
                   ),
                   SizedBox(width: 8),
                   ElevatedButton(
                     onPressed: _addTodo,
                     child: Text('Add'),
                   ),
                 ],
               ),
             ),
             Expanded(
               child: StreamBuilder<List<Document>>(
                 stream: _client.watchCollection('todos'),
                 builder: (context, snapshot) {
                   if (snapshot.hasError) {
                     return Center(child: Text('Error: ${snapshot.error}'));
                   }
                   
                   if (!snapshot.hasData) {
                     return Center(child: CircularProgressIndicator());
                   }
                   
                   final todos = snapshot.data!;
                   
                   if (todos.isEmpty) {
                     return Center(child: Text('No todos yet. Add one above!'));
                   }
                   
                   return ListView.builder(
                     itemCount: todos.length,
                     itemBuilder: (context, index) {
                       final todo = todos[index];
                       final data = todo.data;
                       
                       return Card(
                         child: ListTile(
                           leading: Checkbox(
                             value: data['completed'] ?? false,
                             onChanged: (value) {
                               _client.saveDocument(
                                 'todos',
                                 todo.id,
                                 { ...data, 'completed': value },
                               );
                             },
                           ),
                           title: Text(
                             data['text'] ?? '',
                             style: TextStyle(
                               decoration: (data['completed'] ?? false)
                                   ? TextDecoration.lineThrough
                                   : null,
                             ),
                           ),
                           subtitle: Text(
                             'Created: ${data['createdAt'] ?? 'Unknown'}',
                             style: Theme.of(context).textTheme.bodySmall,
                           ),
                           trailing: IconButton(
                             icon: Icon(Icons.delete, color: Colors.red),
                             onPressed: () {
                               _client.deleteDocument('todos', todo.id);
                             },
                           ),
                         ),
                       );
                     },
                   );
                 },
               ),
             ),
           ],
         ),
       );
     }
     
     @override
     void dispose() {
       _textController.dispose();
       super.dispose();
     }
   }
   ```

5. **iOS/Android Platform Configuration:**

   **iOS Configuration:**
   ```bash
   cd ios && pod install
   ```
   - Add to `ios/Podfile`:
   ```ruby
   pod 'SoliDBClient', '~> 1.0'
   ```
   - Enable Keychain Sharing in Xcode:
     - Open `ios/Runner.xcworkspace`
     - Select Runner → Capabilities → Keychain Sharing → ON
   - Add Background Modes:
     - Capabilities → Background Modes → Enable "Background fetch" and "Background processing"
   - Add to `ios/Runner/Info.plist`:
   ```xml
   <key>UIBackgroundModes</key>
   <array>
     <string>fetch</string>
     <string>processing</string>
   </array>
   ```

   **Android Configuration:**
   - Add to `android/app/build.gradle`:
   ```gradle
   android {
       defaultConfig {
           minSdkVersion 21
       }
   }
   
   dependencies {
       implementation 'com.solidb:client:1.0.0'
   }
   ```
   - Add to `android/app/src/main/AndroidManifest.xml`:
   ```xml
   <uses-permission android:name="android.permission.INTERNET" />
   <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
   <uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />
   
   <application
       android:name=".MainApplication"
       android:label="solidb_todos">
       
       <!-- Background sync service -->
       <service
           android:name="com.solidb.client.SyncService"
           android:enabled="true"
           android:exported="false"
           android:permission="android.permission.BIND_JOB_SERVICE" />
       
       <receiver android:name="com.solidb.client.BootReceiver"
           android:enabled="true"
           android:exported="true">
           <intent-filter>
               <action android:name="android.intent.action.BOOT_COMPLETED" />
           </intent-filter>
       </receiver>
   </application>
   ```

See full Flutter example in `clients/flutter-example/`

## Cross-Platform

When building cross-platform mobile apps, choosing between React Native and Flutter depends on your specific needs:

### Choose React Native when:

| Factor | Recommendation |
|--------|----------------|
| **Team Skills** | Your team already knows JavaScript/React |
| **Existing Code** | You have a web app to share code with |
| **Native Modules** | You need many third-party native libraries |
| **Hiring** | Easier to find JavaScript developers |
| **Iteration Speed** | Hot reload with familiar web patterns |

### Choose Flutter when:

| Factor | Recommendation |
|--------|----------------|
| **Performance** | Need 60fps animations and smooth UI |
| **UI Consistency** | Want pixel-perfect iOS/Android parity |
| **Custom UI** | Complex custom animations and graphics |
| **Dart Preference** | Team prefers strongly-typed languages |
| **Single Codebase** | True "write once, run anywhere" experience |

### Performance Comparison

| Metric | React Native | Flutter |
|--------|-------------|---------|
| Startup Time | ~2-3s | ~1-2s |
| Sync Latency | ~50-100ms | ~30-80ms |
| Memory Usage | Higher (JS bridge) | Lower (compiled) |
| Offline Storage | 50K+ docs | 100K+ docs |
| Bundle Size | +2-3MB | +3-5MB |

### Both SDKs Support:

- ✅ **Hooks/Widgets** - React hooks vs Flutter widgets
- ✅ **Streams/Reactive** - React useSync hook vs Flutter Streams
- ✅ **Offline-First** - Identical SQLite backend via Rust
- ✅ **Auto Sync** - Background sync on both platforms
- ✅ **Type Safety** - TypeScript and Dart type safety
- ✅ **Conflict Resolution** - All strategies available
- ✅ **Background Sync** - iOS background fetch & Android JobScheduler

### Migration Path

Both SDKs share the same Rust core, making it possible to:
1. Start with one framework
2. Gradually migrate screens/components
3. Share sync state between React Native and Flutter modules

See `clients/cross-platform-guide/` for detailed migration strategies.

## API Reference

### SyncManager

The main interface for all sync operations.

#### Lifecycle
- `start()` - Start background sync
- `stop()` - Stop sync manager
- `isOnline()` - Check network status
- `setOnline(online: Bool)` - Set online/offline mode

#### Document Operations
- `saveDocument(collection, key, data)` - Save/update document
- `getDocument(collection, key)` - Retrieve document
- `deleteDocument(collection, key)` - Delete document
- `queryDocuments(collection)` - Query all documents in collection

#### Sync Control
- `syncNow()` - Trigger manual sync
- `getPendingCount()` - Get number of pending changes
- `subscribeCollection(collection)` - Subscribe to collection
- `unsubscribeCollection(collection)` - Unsubscribe from collection

#### Conflict Resolution
- `getConflicts()` - Get list of conflicts
- `resolveConflict(key, resolution, mergedData?)` - Resolve conflict

### Configuration

```rust
SyncConfig {
    deviceId: String,           // Unique device identifier
    serverUrl: Option<String>,  // Server URL (optional for offline-only)
    apiKey: Option<String>,     // API key for authentication
    collections: Option<Vec<String>>, // Collections to sync
    syncIntervalSecs: u64,      // Sync interval (default: 30s)
    maxRetries: u64,            // Max retry attempts (default: 5)
    autoSync: bool,             // Enable automatic sync (default: true)
}
```

## Offline-First Best Practices

### 1. Always Check for Conflicts

```swift
// After sync, check for conflicts
let conflicts = syncManager.getConflicts()
for conflict in conflicts {
    // Show conflict resolution UI
    showConflictDialog(conflict)
}
```

### 2. Handle Network Changes

```swift
// Monitor network status
NotificationCenter.default.addObserver(
    forName: .reachabilityChanged,
    object: nil,
    queue: .main
) { _ in
    let online = reachability.connection != .unavailable
    syncManager.setOnline(online: online)
}
```

### 3. Encrypt Sensitive Data

```swift
// Enable encryption in config
let config = SyncConfig(
    // ... other options
    enableEncryption: true,
    encryptionKey: getEncryptionKey()
)
```

### 4. Use Subscriptions Wisely

```swift
// Only sync collections you need
syncManager.subscribeCollection(collection: "todos")
syncManager.subscribeCollection(collection: "settings")
// Don't sync: "logs", "analytics", etc.
```

## Error Handling

The SDK uses these error types:

- `NetworkError` - Connection issues
- `DatabaseError` - SQLite/local storage errors
- `ConflictError` - Concurrent modification conflicts
- `AuthError` - Authentication failures
- `NotFound` - Document not found
- `InvalidData` - Invalid JSON/data format

```swift
do {
    try syncManager.saveDocument(collection: "todos", key: "1", data: json)
} catch let error as SyncError {
    switch error {
    case .NetworkError(let msg):
        print("Network error: \(msg)")
    case .ConflictError:
        print("Conflict detected - needs resolution")
    default:
        print("Error: \(error)")
    }
}
```

## Performance Tips

1. **Batch operations** - Group multiple saves before syncing
2. **Selective sync** - Only subscribe to needed collections
3. **Delta sync** - Automatically enabled - only changed fields sync
4. **Background sync** - Use autoSync with appropriate intervals
5. **Lazy loading** - Query documents only when needed

## Building from Source

### Requirements
- Rust 1.70+
- Xcode 14+ (for iOS)
- Android Studio Hedgehog+ (for Android)
- UniFFI 0.28+

### Build Commands

```bash
# Full build script
./scripts/build-mobile.sh

# Manual steps:
# 1. Build Rust library
cd clients/rust-client
cargo build --release --features mobile

# 2. Generate bindings
uniffi-bindgen generate src/solidb_client.udl --language kotlin --out-dir ../mobile-bindings/kotlin
uniffi-bindgen generate src/solidb_client.udl --language swift --out-dir ../mobile-bindings/swift

# 3. Create iOS framework
./scripts/build-ios-framework.sh

# 4. Create Android AAR
./scripts/build-android-aar.sh
```

## Troubleshooting

### iOS: "Library not found"
- Ensure framework is embedded (not just linked)
- Check `SoliDBClient.xcframework` is in "Embed Frameworks" build phase

### Android: "UnsatisfiedLinkError"
- Verify AAR includes native libraries for your ABI
- Check `jniLibs` directory has `.so` files

### Sync not working
- Verify `serverUrl` is correct
- Check `apiKey` has sync permissions
- Ensure collections are subscribed
- Check network connectivity

## License

MIT License - See LICENSE file for details

## Support

- Documentation: https://solidb.io/docs/mobile
- GitHub Issues: https://github.com/solisoft/solidb/issues
- Discord: https://discord.gg/solidb
