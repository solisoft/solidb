# Timeseries explorer - per-collection chart (TIME_BUCKET aggregations),
# live-tail polling, raw points table and retention controls (TTL indexes +
# manual prune) for timeseries collections (insert-only, UUIDv7-keyed).

class TimeseriesController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/collections/:name/timeseries
  def show
    this._ctx()
    return redirect(db_collection_docs_path(@db, @collection_name)) if @collection_type != "timeseries"
    @title = @collection_name + " · timeseries · " + @db
    this._reset_banners()
    this._discover_fields()
    this._load_ttl()
    this._load_points()
  end

  # GET /databases/:db/collections/:name/timeseries/data
  # uPlot-ready JSON: { ok, series: ["time", label...], data: [[x...], [y...]...] }
  # with dense, gap-filled aligned arrays. Also serves the live-tail polls.
  def data
    this._ctx()
    return this._json_error(404, "not a timeseries collection", false) if @collection_type != "timeseries"
    this._reset_banners()
    this._discover_fields()
    return this._json_error(422, @field_error, @solidb_down) unless @field_error.blank?
    chart = this._chart_params()
    return this._json_error(422, chart["error"], false) unless chart["error"].blank?

    bind_vars = { "from": chart["from_ms"], "to": chart["to_ms"], "bucket_ms": chart["bucket_ms"] }
    value_label = chart["agg"] == "count" ? "count" : chart["value_field"]
    labels = [value_label]
    series_capped = false
    if !chart["group_by"].blank?
      probe = this._series_probe(chart)
      return this._json_error(502, probe["error"], probe["down"]) unless probe["error"].blank?
      series_capped = probe["capped"]
      labels = probe["names"].map do |name| str(name) end
      bind_vars["series"] = probe["names"]
    end

    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db),
                                   { "query": this._bucket_query(chart), "bindVars": bind_vars, "cache": false })
    if !result["ok"]
      return this._json_error(502, result["error"] ?? "request failed", (result["status"] ?? -1) == 0)
    end
    rows = (result["data"] ?? {})["result"] ?? []
    return render_json(this._chart_payload(chart, rows, labels, series_capped))
  end

  # GET /databases/:db/collections/:name/timeseries/points - HTMX fragment,
  # newest-first raw documents inside the chart window.
  def points
    this._ctx()
    return redirect(db_collection_docs_path(@db, @collection_name)) if @collection_type != "timeseries"
    this._reset_banners()
    this._discover_fields()
    this._load_points()
    return render("timeseries/_points", { "layout": false })
  end

  # POST /databases/:db/collections/:name/timeseries/ttl
  def create_ttl
    this._ctx()
    field = (params["field"] ?? "").trim()
    if !this._safe_field(field)
      return this._respond({ "ok": false, "status": 422, "error": "ttl field is required" }, "")
    end
    expire_seconds = (params["expire_after_seconds"] ?? "").trim().to_int()
    if expire_seconds <= 0
      error = "expire_after_seconds must be a positive integer"
      return this._respond({ "ok": false, "status": 422, "error": error }, "")
    end
    index_name = (params["index_name"] ?? "").trim()
    index_name = "ttl_" + field if index_name.blank?
    result = SolidbClient.post_api(SolidbEndpoints.ttl_indexes(@db, @collection_name),
                                   { "name": index_name, "field": field,
                                     "expire_after_seconds": expire_seconds })
    return this._respond(result, "ttl index " + index_name + " created — expires after " +
                                 str(expire_seconds) + "s")
  end

  # DELETE /databases/:db/collections/:name/timeseries/ttl/:index_name
  def delete_ttl
    this._ctx()
    index_name = params["index_name"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.ttl_index(@db, @collection_name, index_name))
    return this._respond(result, "ttl index " + index_name + " dropped")
  end

  # POST /databases/:db/collections/:name/timeseries/prune - bulk-delete by
  # INSERT time (the UUIDv7 key embeds it, so this is what the server prunes
  # by); individual deletes are forbidden on timeseries collections.
  def prune
    this._ctx()
    older_than = this._prune_cutoff()
    if older_than.blank?
      error = "prune needs a cutoff — pick a preset or give an ISO8601 timestamp"
      return this._respond({ "ok": false, "status": 422, "error": error }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.collection_prune(@db, @collection_name),
                                   { "older_than": older_than })
    deleted = (result["data"] ?? {})["deleted"] ?? 0
    return this._respond(result, "pruned " + str(deleted) + " point(s) inserted before " + older_than)
  end

  def _ctx
    @db = params["db"] ?? ""
    @collection_name = params["name"] ?? ""
    @databases = AdminContext.database_names()
    this._load_collection_type()
  end

  def _load_collection_type
    result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    collections = (result["data"] ?? {})["collections"] ?? []
    matching = collections.filter do |coll| coll["name"] == @collection_name end
    @collection_type = matching.length() > 0 ? (matching[0]["type"] ?? "document") : "document"
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  # Retention actions land here: banner + reload, full page or HTMX swap of
  # #content (same shape as documents#_respond).
  def _respond(result, notice)
    @title = @collection_name + " · timeseries · " + @db
    this._reset_banners()
    this._discover_fields()
    this._load_ttl()
    this._load_points()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    return render("timeseries/show", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("timeseries/show")
  end

  # --- field discovery -----------------------------------------------------

  # Sample the newest documents to infer the time field (+ its encoding), the
  # numeric value fields and the group-by candidates. UUIDv7 keys sort by
  # insert time, so SORT d._key DESC reads newest-first without an index.
  def _discover_fields
    @time_field = ""
    @time_kind = "ms"
    @time_field_candidates = []
    @value_fields = []
    @group_fields = []
    @field_error = ""

    query = "FOR d IN " + @collection_name + " SORT d._key DESC LIMIT @n RETURN d"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db),
                                   { "query": query, "bindVars": { "n": 50 }, "cache": false })
    if !result["ok"]
      @field_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
      return nil
    end
    docs = (result["data"] ?? {})["result"] ?? []
    if docs.length() == 0
      @field_error = "no documents yet — insert points to explore them here"
      return nil
    end

    counts = { "numeric": {}, "string": {}, "time_ms": {}, "time_s": {}, "time_iso": {} }
    fields = []
    for doc in docs
      this._tally_fields(doc, counts, fields)
    end
    this._pick_fields(docs.length() / 2, counts, fields)
  end

  # One sampled document's contribution to the field tallies.
  def _tally_fields(doc, counts, fields)
    system_attrs = ["_id", "_key", "_rev", "_created_at", "_updated_at"]
    for field in doc.keys()
      if !system_attrs.includes?(field) && this._safe_field(field)
        fields.push(field) unless fields.includes?(field)
        this._tally_value(field, doc[field], counts)
      end
    end
  end

  def _tally_value(field, value, counts)
    kind = type(value)
    if kind == "int" || kind == "float"
      this._bump(counts["numeric"], field)
      # epoch heuristics: > ~1973 in ms is a ms timestamp, in seconds a
      # seconds timestamp. Plain metric values land below both cutoffs.
      this._bump(counts["time_ms"], field) if value > 100000000000
      this._bump(counts["time_s"], field) if value > 100000000 && value <= 100000000000
    end
    if kind == "string"
      this._bump(counts["string"], field)
      this._bump(counts["time_iso"], field) if this._looks_iso(value)
    end
    this._bump(counts["string"], field) if kind == "bool"
  end

  def _bump(table, field)
    table[field] = (table[field] ?? 0) + 1
  end

  # "2026-06-07T10:00:00Z"-style — a cheap shape check is enough on a sample.
  def _looks_iso(text)
    return text.gsub("^[0-9]{4}-[0-9]{2}-[0-9]{2}[T ].*$", "") == ""
  end

  # Turn the tallies into @time_field/@time_kind/@value_fields/@group_fields.
  # "Majority" = more than half the sampled docs agree on the field's shape.
  def _pick_fields(half, counts, fields)
    for field in fields
      time_hits = (counts["time_ms"][field] ?? 0) + (counts["time_s"][field] ?? 0) +
                  (counts["time_iso"][field] ?? 0)
      @time_field_candidates.push(field) if time_hits > half
    end
    @time_field = "timestamp" if @time_field_candidates.includes?("timestamp")
    @time_field = @time_field_candidates[0] ?? "" if @time_field.blank?
    if @time_field.blank?
      @field_error = "no timestamp-like field found in recent documents"
      return nil
    end
    second_hits = counts["time_s"][@time_field] ?? 0
    ms_hits = counts["time_ms"][@time_field] ?? 0
    iso_hits = counts["time_iso"][@time_field] ?? 0
    @time_kind = "ms"
    @time_kind = "s" if second_hits > ms_hits
    @time_kind = "iso" if iso_hits > 0

    for field in fields
      if field != @time_field
        numeric_hits = counts["numeric"][field] ?? 0
        string_hits = counts["string"][field] ?? 0
        @value_fields.push(field) if numeric_hits > half
        @group_fields.push(field) if string_hits > half
      end
    end
    @field_error = "no numeric value fields found in recent documents" if @value_fields.length() == 0
  end

  # Field names get spliced into query text (only values can be bind vars),
  # so only identifier-shaped names are ever offered or accepted.
  def _safe_field(name)
    return false if name.blank?
    return name.gsub("^[A-Za-z_][A-Za-z0-9_]*$", "") == ""
  end

  # --- chart params --------------------------------------------------------

  # range/from/to -> epoch-ms window. Shared by the chart and the points table.
  def _window_params
    range_seconds = { "15m": 900, "1h": 3600, "6h": 21600, "24h": 86400, "7d": 604800, "30d": 2592000 }
    range_key = (params["range"] ?? "1h").trim()
    to_ms = DateTime.now().to_unix() * 1000
    from_ms = 0
    if range_key == "custom"
      from_ms = this._parse_time(params["from"] ?? "")
      to_ms = this._parse_time(params["to"] ?? "")
      if from_ms.nil? || to_ms.nil? || from_ms >= to_ms
        return { "error": "custom range needs from/to (ISO8601 or epoch ms) with from < to" }
      end
    else
      seconds = range_seconds[range_key]
      return { "error": "unknown range " + range_key } if seconds.nil?
      from_ms = to_ms - seconds * 1000
    end
    return { "error": "", "from_ms": from_ms, "to_ms": to_ms, "range_key": range_key }
  end

  # Window + bucket + aggregation + fields, all validated. Returns
  # { "error": "..." } on the first invalid input.
  def _chart_params
    window = this._window_params()
    return window unless window["error"].blank?
    from_ms = window["from_ms"]
    to_ms = window["to_ms"]

    agg = (params["agg"] ?? "avg").trim()
    agg_fns = { "avg": "AVG", "min": "MIN", "max": "MAX", "sum": "SUM", "count": "COUNT" }
    agg_fn = agg_fns[agg]
    return { "error": "unknown aggregation " + agg } if agg_fn.nil?

    value_field = (params["value_field"] ?? "").trim()
    if agg != "count"
      value_field = @value_fields[0] ?? "" if value_field.blank?
      if !@value_fields.includes?(value_field)
        return { "error": "unknown value field " + value_field }
      end
    end

    group_by = (params["group_by"] ?? "").trim()
    if !group_by.blank? && !@group_fields.includes?(group_by)
      return { "error": "unknown group field " + group_by }
    end

    bucket_key = (params["bucket"] ?? "auto").trim()
    range_ms = to_ms - from_ms
    bucket_key = this._auto_bucket(range_ms) if bucket_key == "auto"
    return { "error": "unknown bucket " + bucket_key } if this._bucket_seconds(bucket_key).nil?

    # Widen the bucket until the range fits MAX_BUCKETS slots — a 30d range
    # at 1m buckets would be 43k points.
    effective = ""
    reached = false
    for candidate in this._bucket_ladder()
      reached = true if candidate == bucket_key
      if reached && effective.blank?
        effective = candidate if range_ms / (this._bucket_seconds(candidate) * 1000) <= 1000
      end
    end
    effective = "1d" if effective.blank?

    return {
      "error": "",
      "from_ms": from_ms, "to_ms": to_ms,
      "bucket_key": effective, "bucket_ms": this._bucket_seconds(effective) * 1000,
      "agg": agg, "agg_fn": agg_fn,
      "value_field": value_field, "group_by": group_by,
      "capped": effective != bucket_key
    }
  end

  def _bucket_ladder
    return ["1m", "5m", "15m", "1h", "6h", "1d"]
  end

  def _bucket_seconds(key)
    table = { "1m": 60, "5m": 300, "15m": 900, "1h": 3600, "6h": 21600, "1d": 86400 }
    return table[key]
  end

  # Smallest ladder interval that keeps the range at ~200 buckets or fewer.
  def _auto_bucket(range_ms)
    for candidate in this._bucket_ladder()
      return candidate if range_ms / (this._bucket_seconds(candidate) * 1000) <= 200
    end
    return "1d"
  end

  # ISO8601 ("2026-06-07T10:00:00Z") or epoch milliseconds -> ms; nil when
  # unparseable.
  def _parse_time(raw)
    text = (raw ?? "").trim()
    return nil if text.blank?
    return text.to_int() if text.gsub("^[0-9]+$", "") == ""
    seconds = DateTime.parse(text).to_unix() rescue nil
    return nil if seconds.nil?
    return seconds * 1000
  end

  # --- queries -------------------------------------------------------------

  # Normalize the time field to epoch ms inside the query, whatever the docs
  # store: ms numbers pass through, second numbers are scaled, ISO strings go
  # through DATE_TIMESTAMP.
  def _time_expr
    field_expr = "d." + @time_field
    return "DATE_TIMESTAMP(" + field_expr + ")" if @time_kind == "iso"
    return "TO_NUMBER(" + field_expr + ") * 1000" if @time_kind == "s"
    return "TO_NUMBER(" + field_expr + ")"
  end

  # Collection, field and function names are spliced (only values can be bind
  # vars) — every one of them is validated against the discovered field sets
  # or internal whitelists before reaching this point.
  #
  # Buckets use explicit FLOOR arithmetic rather than TIME_BUCKET: the scale
  # normalization (seconds * 1000) makes the expression a float, and
  # TIME_BUCKET only accepts integer timestamps.
  def _bucket_query(chart)
    time_expr = this._time_expr()
    agg_expr = "COUNT()"
    agg_expr = chart["agg_fn"] + "(TO_NUMBER(d." + chart["value_field"] + "))" if chart["agg"] != "count"
    query = "FOR d IN " + @collection_name +
            " FILTER d." + @time_field + " != null" +
            " AND " + time_expr + " >= @from AND " + time_expr + " < @to"
    collect = " COLLECT bucket = FLOOR((" + time_expr + ") / @bucket_ms) * @bucket_ms"
    if chart["group_by"].blank?
      return query + collect + " AGGREGATE v = " + agg_expr +
             " SORT bucket ASC RETURN { \"bucket\": bucket, \"v\": v }"
    end
    query = query + " AND d." + chart["group_by"] + " IN @series"
    collect = collect + ", series = d." + chart["group_by"]
    return query + collect + " AGGREGATE v = " + agg_expr +
           " SORT bucket ASC RETURN { \"bucket\": bucket, \"series\": series, \"v\": v }"
  end

  # Distinct group values inside the window, capped at 10 series (+1 fetched
  # to detect overflow). The main query binds the list so its row count stays
  # bounded at buckets × series even on high-cardinality fields.
  def _series_probe(chart)
    time_expr = this._time_expr()
    query = "FOR d IN " + @collection_name +
            " FILTER d." + @time_field + " != null" +
            " AND " + time_expr + " >= @from AND " + time_expr + " < @to" +
            " COLLECT series = d." + chart["group_by"] +
            " LIMIT 11 RETURN series"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db),
                                   { "query": query,
                                     "bindVars": { "from": chart["from_ms"], "to": chart["to_ms"] },
                                     "cache": false })
    if !result["ok"]
      return { "error": result["error"] ?? "request failed", "down": (result["status"] ?? -1) == 0 }
    end
    names = (result["data"] ?? {})["result"] ?? []
    capped = names.length() > 10
    names = names.slice(0, 10) if capped
    return { "error": "", "names": names, "capped": capped }
  end

  # Dense, gap-filled aligned arrays — every series carries one value (or
  # null) per bucket slot, the shape uPlot requires. X is in SECONDS.
  def _chart_payload(chart, rows, labels, series_capped)
    bucket_ms = chart["bucket_ms"]
    aligned_from = (chart["from_ms"] / bucket_ms) * bucket_ms
    slot_count = ((chart["to_ms"] - aligned_from) + bucket_ms - 1) / bucket_ms

    xs = []
    slot = 0
    while slot < slot_count
      xs.push((aligned_from + slot * bucket_ms) / 1000)
      slot = slot + 1
    end

    columns = []
    label_slots = {}
    position = 0
    for label in labels
      column = []
      fill = 0
      while fill < slot_count
        column.push(nil)
        fill = fill + 1
      end
      columns.push(column)
      label_slots[label] = position
      position = position + 1
    end

    for row in rows
      slot_index = int((row["bucket"] - aligned_from) / bucket_ms)
      if slot_index >= 0 && slot_index < slot_count
        column_index = chart["group_by"].blank? ? 0 : (label_slots[str(row["series"])] ?? -1)
        columns[column_index][slot_index] = row["v"] if column_index >= 0
      end
    end

    series_labels = ["time"]
    data_arrays = [xs]
    for label in labels
      series_labels.push(label)
    end
    for column in columns
      data_arrays.push(column)
    end

    value_label = chart["agg"] == "count" ? "count" : chart["value_field"]
    return {
      "ok": true,
      "time_field": @time_field,
      "value_field": value_label,
      "agg": chart["agg"],
      "bucket": chart["bucket_key"],
      "from": chart["from_ms"],
      "to": chart["to_ms"],
      "capped": chart["capped"],
      "series_capped": series_capped,
      "series": series_labels,
      "data": data_arrays
    }
  end

  # --- points table --------------------------------------------------------

  def _load_points
    @points = []
    @has_more = false
    @points_error = ""
    @limit = this._page_limit()
    offset = (params["offset"] ?? "0").to_int()
    offset = 0 if offset < 0
    @offset = offset
    window = this._window_params()
    if !window["error"].blank?
      @points_error = window["error"]
      return nil
    end
    @from_ms = window["from_ms"]
    @to_ms = window["to_ms"]
    @range_key = window["range_key"]
    return nil unless @field_error.blank?

    time_expr = this._time_expr()
    query = "FOR d IN " + @collection_name +
            " FILTER d." + @time_field + " != null" +
            " AND " + time_expr + " >= @from AND " + time_expr + " < @to" +
            " SORT d._key DESC LIMIT @offset, @batch RETURN d"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db),
                                   { "query": query,
                                     "bindVars": { "from": @from_ms, "to": @to_ms,
                                                   "offset": @offset, "batch": @limit + 1 },
                                     "cache": false })
    if !result["ok"]
      @points_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
      return nil
    end
    rows = (result["data"] ?? {})["result"] ?? []
    @has_more = rows.length() > @limit
    @points = @has_more ? rows.slice(0, @limit) : rows
  end

  def _page_limit
    limit = (params["limit"] ?? "25").to_int()
    return 25 unless [25, 50, 100].includes?(limit)
    return limit
  end

  # --- retention -----------------------------------------------------------

  def _load_ttl
    result = SolidbClient.get_api(SolidbEndpoints.ttl_indexes(@db, @collection_name))
    @ttl_indexes = (result["data"] ?? {})["indexes"] ?? []
  end

  # Relative preset ("30d") or explicit ISO timestamp -> ISO8601 cutoff.
  # Blank when neither parses.
  def _prune_cutoff
    presets = { "7d": 604800, "30d": 2592000, "90d": 7776000 }
    preset = (params["older_than_preset"] ?? "").trim()
    if !presets[preset].nil?
      cutoff_seconds = DateTime.now().to_unix() - presets[preset]
      return DateTime.from_unix(cutoff_seconds).to_iso()
    end
    custom = (params["older_than"] ?? "").trim()
    return "" if custom.blank?
    parsed = DateTime.parse(custom) rescue nil
    return "" if parsed.nil?
    return parsed.to_iso()
  end

  # JSON error with a real HTTP status (render_json always answers 200).
  def _json_error(status, message, down)
    body = { "ok": false, "error": message ?? "request failed", "solidb_down": down ?? false }
    return {
      "status": status,
      "headers": { "Content-Type": "application/json" },
      "body": JSON.stringify(body)
    }
  end
end
