module SoliDB
  class ColumnarClient
    def initialize(client)
      @client = client
    end

    def create(name, columns)
      @client.send_command("create_columnar_table", database: @client.database, name: name, columns: columns)
    end

    def list
      @client.send_command("list_columnar_tables", database: @client.database) || []
    end

    def get(name)
      @client.send_command("get_columnar_table", database: @client.database, name: name)
    end

    def delete(name)
      @client.send_command("delete_columnar_table", database: @client.database, name: name)
      nil
    end

    def insert(name, rows)
      @client.send_command("columnar_insert", database: @client.database, name: name, rows: rows)
    end

    def query(name, query, params: nil)
      args = { database: @client.database, name: name, query: query }
      args[:params] = params if params
      @client.send_command("columnar_query", args) || []
    end

    def aggregate(name, aggregation)
      @client.send_command("columnar_aggregate", database: @client.database, name: name, aggregation: aggregation)
    end

    def create_index(table_name, index_name:, column:, index_type: nil)
      args = {
        database: @client.database,
        table_name: table_name,
        index_name: index_name,
        column: column
      }
      args[:index_type] = index_type if index_type
      @client.send_command("columnar_create_index", args)
    end

    def list_indexes(table_name)
      @client.send_command("columnar_list_indexes", database: @client.database, table_name: table_name) || []
    end

    def delete_index(table_name, index_name)
      @client.send_command("columnar_delete_index", database: @client.database, table_name: table_name, index_name: index_name)
      nil
    end

    def add_column(table_name, column_name:, column_type:, default_value: nil)
      args = {
        database: @client.database,
        table_name: table_name,
        column_name: column_name,
        column_type: column_type
      }
      args[:default_value] = default_value if default_value
      @client.send_command("columnar_add_column", args)
      nil
    end

    def drop_column(table_name, column_name)
      @client.send_command("columnar_drop_column", database: @client.database, table_name: table_name, column_name: column_name)
      nil
    end

    def stats(table_name)
      @client.send_command("columnar_stats", database: @client.database, table_name: table_name)
    end
  end
end
