# Live queries - stream the SoliDB changefeed over a WebSocket the browser
# opens directly against the server. Tokens are short-lived, so the page
# fetches a fresh one from /databases/:db/live/token on every (re)connect.

class LiveQueriesController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/live
  def show
    this._ctx()
    @title = "Live · " + @db
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
    collections_result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    @collections = (collections_result["data"] ?? {})["collections"] ?? []
    @collection_names = @collections.map do |coll| coll["name"] ?? "" end
  end

  # GET /databases/:db/live/token - JSON consumed by the page's JS
  def token
    this._ctx()
    live_token = SolidbClient.livequery_token()
    if live_token.blank?
      return render_json({ "ok": false, "error": "could not mint a livequery token (is SoliDB up?)" })
    end
    return render_json({
      "ok": true,
      "token": live_token,
      "ws_url": SolidbClient.public_ws_url() + "/_api/ws/changefeed",
      "database": @db
    })
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end
end
