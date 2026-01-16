module SoliDB
  class JobsClient
    def initialize(client)
      @client = client
    end

    def list_queues
      @client.send_command("list_queues", database: @client.database) || []
    end

    def list_jobs(queue_name, status: nil, limit: nil, offset: nil)
      params = { database: @client.database, queue_name: queue_name }
      params[:status] = status if status
      params[:limit] = limit if limit
      params[:offset] = offset if offset
      @client.send_command("list_jobs", params) || []
    end

    def enqueue(queue_name, script_path:, params: nil, priority: nil, run_at: nil)
      args = {
        database: @client.database,
        queue_name: queue_name,
        script_path: script_path
      }
      args[:params] = params if params
      args[:priority] = priority if priority
      args[:run_at] = run_at if run_at
      @client.send_command("enqueue_job", args)
    end

    def cancel(job_id)
      @client.send_command("cancel_job", database: @client.database, job_id: job_id)
      nil
    end

    def get(job_id)
      @client.send_command("get_job", database: @client.database, job_id: job_id)
    end
  end
end
