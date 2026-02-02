import SwiftUI
import SoliDBClient

// MARK: - Data Models

struct Todo: Identifiable, Codable {
    let id: String
    var text: String
    var completed: Bool
    var createdAt: Date
}

// MARK: - View Model

@MainActor
class TodoViewModel: ObservableObject {
    @Published var todos: [Todo] = []
    @Published var newTodoText = ""
    @Published var isOnline = false
    @Published var pendingCount: UInt64 = 0
    @Published var isSyncing = false
    @Published var lastSyncTime: String?
    
    private var syncManager: SyncManager?
    
    init() {
        setupSyncManager()
    }
    
    private func setupSyncManager() {
        let config = SyncConfig(
            deviceId: Utils.generateDeviceId(),
            serverUrl: "https://your-server.com:6745",
            apiKey: "your-api-key",
            collections: ["todos"],
            syncIntervalSecs: 30,
            maxRetries: 5,
            autoSync: true
        )
        
        do {
            syncManager = try SyncManager(config: config)
            syncManager?.start()
            loadTodos()
            updateStats()
        } catch {
            print("Failed to initialize sync manager: \(error)")
        }
    }
    
    func loadTodos() {
        guard let manager = syncManager else { return }
        
        do {
            let documents = try manager.queryDocuments(collection: "todos")
            todos = documents.compactMap { doc in
                guard let data = doc.data.data(using: .utf8) else { return nil }
                return try? JSONDecoder().decode(Todo.self, from: data)
            }
        } catch {
            print("Failed to load todos: \(error)")
        }
    }
    
    func addTodo() {
        guard !newTodoText.isEmpty, let manager = syncManager else { return }
        
        let todo = Todo(
            id: UUID().uuidString,
            text: newTodoText,
            completed: false,
            createdAt: Date()
        )
        
        do {
            let jsonData = try JSONEncoder().encode(todo)
            let jsonString = String(data: jsonData, encoding: .utf8)!
            
            try manager.saveDocument(
                collection: "todos",
                key: todo.id,
                data: jsonString
            )
            
            newTodoText = ""
            loadTodos()
            updateStats()
        } catch {
            print("Failed to add todo: \(error)")
        }
    }
    
    func toggleTodo(_ todo: Todo) {
        guard let manager = syncManager else { return }
        
        var updated = todo
        updated.completed.toggle()
        
        do {
            let jsonData = try JSONEncoder().encode(updated)
            let jsonString = String(data: jsonData, encoding: .utf8)!
            
            try manager.saveDocument(
                collection: "todos",
                key: todo.id,
                data: jsonString
            )
            
            loadTodos()
        } catch {
            print("Failed to toggle todo: \(error)")
        }
    }
    
    func deleteTodo(_ todo: Todo) {
        guard let manager = syncManager else { return }
        
        do {
            try manager.deleteDocument(collection: "todos", key: todo.id)
            loadTodos()
            updateStats()
        } catch {
            print("Failed to delete todo: \(error)")
        }
    }
    
    func syncNow() {
        guard let manager = syncManager, !isSyncing else { return }
        
        isSyncing = true
        
        Task {
            do {
                let result = try manager.syncNow()
                await MainActor.run {
                    self.isSyncing = false
                    self.loadTodos()
                    self.updateStats()
                }
            } catch {
                await MainActor.run {
                    self.isSyncing = false
                    print("Sync failed: \(error)")
                }
            }
        }
    }
    
    func updateStats() {
        guard let manager = syncManager else { return }
        
        isOnline = manager.isOnline()
        pendingCount = manager.getPendingCount()
        lastSyncTime = manager.getLastSyncTime()
    }
}

// MARK: - Views

@main
struct SoliDBTodoApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    @StateObject private var viewModel = TodoViewModel()
    
    var body: some View {
        NavigationView {
            VStack(spacing: 0) {
                // Status bar
                StatusBar(viewModel: viewModel)
                
                // Add new todo
                AddTodoBar(viewModel: viewModel)
                
                // Todo list
                List {
                    ForEach(viewModel.todos) { todo in
                        TodoRow(todo: todo, viewModel: viewModel)
                    }
                    .onDelete { indexSet in
                        indexSet.forEach { index in
                            viewModel.deleteTodo(viewModel.todos[index])
                        }
                    }
                }
                .listStyle(.plain)
            }
            .navigationTitle("Offline Todos")
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(action: { viewModel.syncNow() }) {
                        if viewModel.isSyncing {
                            ProgressView()
                        } else {
                            Image(systemName: viewModel.isOnline ? 
                                  "arrow.clockwise.icloud" : "icloud.slash")
                                .foregroundColor(viewModel.isOnline ? .green : .red)
                        }
                    }
                    .disabled(viewModel.isSyncing)
                }
            }
        }
    }
}

struct StatusBar: View {
    @ObservedObject var viewModel: TodoViewModel
    
    var body: some View {
        HStack {
            // Online status
            HStack(spacing: 4) {
                Circle()
                    .fill(viewModel.isOnline ? Color.green : Color.red)
                    .frame(width: 8, height: 8)
                Text(viewModel.isOnline ? "Online" : "Offline")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            
            Spacer()
            
            // Pending count
            if viewModel.pendingCount > 0 {
                Label(
                    "\(viewModel.pendingCount) pending",
                    systemImage: "arrow.up.arrow.down"
                )
                .font(.caption)
                .foregroundColor(.orange)
            }
            
            Spacer()
            
            // Last sync
            if let lastSync = viewModel.lastSyncTime {
                Text("Synced: \(formatDate(lastSync))")
                    .font(.caption2)
                    .foregroundColor(.secondary)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .background(Color(.systemGray6))
    }
    
    private func formatDate(_ dateString: String) -> String {
        // Simple formatting - in production use proper date parsing
        if dateString.count > 10 {
            let index = dateString.index(dateString.startIndex, offsetBy: 10)
            return String(dateString[..<index])
        }
        return dateString
    }
}

struct AddTodoBar: View {
    @ObservedObject var viewModel: TodoViewModel
    
    var body: some View {
        HStack(spacing: 12) {
            TextField("New todo...", text: $viewModel.newTodoText)
                .textFieldStyle(RoundedBorderTextFieldStyle())
            
            Button(action: { viewModel.addTodo() }) {
                Image(systemName: "plus.circle.fill")
                    .font(.title2)
                    .foregroundColor(.blue)
            }
            .disabled(viewModel.newTodoText.isEmpty)
        }
        .padding()
    }
}

struct TodoRow: View {
    let todo: Todo
    @ObservedObject var viewModel: TodoViewModel
    
    var body: some View {
        HStack {
            Button(action: { viewModel.toggleTodo(todo) }) {
                Image(systemName: todo.completed ? 
                      "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundColor(todo.completed ? .green : .gray)
            }
            .buttonStyle(PlainButtonStyle())
            
            Text(todo.text)
                .strikethrough(todo.completed)
                .foregroundColor(todo.completed ? .secondary : .primary)
            
            Spacer()
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Preview

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
