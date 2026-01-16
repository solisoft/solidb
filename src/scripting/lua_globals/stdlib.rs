//! Standard library extensions for Lua (table, string)

use crate::error::DbError;
use mlua::Lua;

/// Setup enhanced table functions using pure Lua
pub fn setup_table_extensions(lua: &Lua) -> Result<(), DbError> {
    lua.load(
        r#"
        -- table.sorted(t, comp?) - returns sorted copy of table
        function table.sorted(t, comp)
            local copy = {}
            for i, v in ipairs(t) do copy[i] = v end
            table.sort(copy, comp)
            return copy
        end

        -- table.keys(t) - returns array of keys
        function table.keys(t)
            local keys = {}
            for k, _ in pairs(t) do
                keys[#keys + 1] = k
            end
            return keys
        end

        -- table.values(t) - returns array of values
        function table.values(t)
            local values = {}
            for _, v in pairs(t) do
                values[#values + 1] = v
            end
            return values
        end

        -- table.merge(t1, t2) - merge two tables (t2 overwrites t1)
        function table.merge(t1, t2)
            local result = {}
            for k, v in pairs(t1) do result[k] = v end
            for k, v in pairs(t2) do result[k] = v end
            return result
        end

        -- table.filter(t, fn) - filter array elements
        function table.filter(t, fn)
            local result = {}
            for i, v in ipairs(t) do
                if fn(v, i) then
                    result[#result + 1] = v
                end
            end
            return result
        end

        -- table.map(t, fn) - map array elements
        function table.map(t, fn)
            local result = {}
            for i, v in ipairs(t) do
                result[i] = fn(v, i)
            end
            return result
        end

        -- table.find(t, fn) - find first element matching predicate
        function table.find(t, fn)
            for i, v in ipairs(t) do
                if fn(v, i) then
                    return v, i
                end
            end
            return nil
        end

        -- table.contains(t, value) - check if array contains value
        function table.contains(t, value)
            for _, v in ipairs(t) do
                if v == value then return true end
            end
            return false
        end

        -- table.reverse(t) - reverse array
        function table.reverse(t)
            local result = {}
            for i = #t, 1, -1 do
                result[#result + 1] = t[i]
            end
            return result
        end

        -- table.slice(t, start, stop) - slice array
        function table.slice(t, start, stop)
            local result = {}
            start = start or 1
            stop = stop or #t
            for i = start, stop do
                result[#result + 1] = t[i]
            end
            return result
        end

        -- table.len(t) - count elements (works for non-arrays too)
        function table.len(t)
            local count = 0
            for _ in pairs(t) do count = count + 1 end
            return count
        end
    "#,
    )
    .exec()
    .map_err(|e| DbError::InternalError(format!("Failed to setup table extensions: {}", e)))?;

    Ok(())
}
