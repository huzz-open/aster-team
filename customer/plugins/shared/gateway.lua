-- Official protocol rules. Aster owns authentication, account selection,
-- endpoint resolution, network I/O, model authorization and accounting.
local M = {}
local array = aster.array
local public_rules = aster.data("public-rules.json")
local provider = aster.data("provider.json").provider
local channels = aster.data("channels.json")

local reject_unknown

local function ok(value)
  return { status = "ok", value = value }
end

local function failure(code, path, reason)
  return {
    status = "error",
    error = {
      code = code,
      source_path = path or aster.null,
      reason_key = reason or code,
      allowed_values = aster.null,
    },
  }
end

local function is_present(value)
  return value ~= nil and value ~= aster.null
end

local function is_array(value)
  return type(value) == "table" and aster.is_array(value)
end

local function function_name(value)
  return type(value) == "string" and #value > 0 and #value <= 128
      and value:match("^[%w_%-]+$") ~= nil
end

local function item(role, content, call_id, tool_name, arguments)
  return {
    role = role,
    content = content or array({}),
    call_id = call_id or aster.null,
    tool_name = tool_name or aster.null,
    arguments = arguments or aster.null,
  }
end

local function parse_arguments(value, path)
  if type(value) == "string" then
    local success, parsed = pcall(aster.parse_json, value)
    if not success or type(parsed) ~= "table" or is_array(parsed) then
      return nil, failure("invalid_request", path, "tool_arguments_must_be_object")
    end
    return parsed
  end
  if type(value) ~= "table" or is_array(value) then
    return nil, failure("invalid_request", path, "tool_arguments_must_be_object")
  end
  return value
end

local function text_content(value, path)
  if type(value) == "string" then
    return array({ { kind = "text", text = value } })
  end
  if type(value) ~= "table" then
    return nil, failure("unsupported_parameter", path, "content_must_be_text")
  end
  if not is_array(value) then
    return nil, failure("unsupported_parameter", path, "content_array_required")
  end
  local parts = array({})
  for index, part in ipairs(value) do
    if type(part) ~= "table" then
      return nil, failure("unsupported_parameter", path .. "[" .. index .. "]", "unsupported_content_part")
    end
    local unknown = reject_unknown(part, { type = true, text = true })
    if unknown then return nil, unknown end
    local kind = part.type
    if kind == "text" or kind == "input_text" or kind == "output_text" then
      if type(part.text) ~= "string" then
        return nil, failure("unsupported_parameter", path .. "[" .. index .. "].text", "text_required")
      end
      parts[#parts + 1] = { kind = "text", text = part.text }
    else
      return nil, failure("unsupported_parameter", path .. "[" .. index .. "].type", "content_part_not_supported")
    end
  end
  return parts
end

local function append_message(conversation, message, path)
  if type(message) ~= "table" or type(message.role) ~= "string" then
    return failure("unsupported_parameter", path, "message_invalid")
  end
  if message.role ~= "system" and message.role ~= "developer"
      and message.role ~= "user" and message.role ~= "assistant"
      and message.role ~= "tool" then
    return failure("unsupported_parameter", path .. ".role", "role_not_supported")
  end
  local unknown = reject_unknown(message, {
    type = true, role = true, content = true,
    tool_calls = true, tool_call_id = true,
  })
  if unknown then return unknown end
  if is_present(message.type) and message.type ~= "message" then
    return failure("unsupported_parameter", path .. ".type", "message_item_required")
  end
  if message.role == "tool" then
    if type(message.tool_call_id) ~= "string" or message.tool_call_id == "" then
      return failure("invalid_request", path .. ".tool_call_id", "tool_call_id_required")
    end
    local content, err = text_content(message.content, path .. ".content")
    if err then return err end
    conversation[#conversation + 1] = item("tool", content, message.tool_call_id)
    return nil
  end
  if is_present(message.tool_call_id) then
    return failure("invalid_request", path .. ".tool_call_id", "tool_result_role_required")
  end
  if is_present(message.content) then
    local content, err = text_content(message.content, path .. ".content")
    if err then return err end
    conversation[#conversation + 1] = item(message.role, content)
  elseif not is_present(message.tool_calls) then
    return failure("invalid_request", path .. ".content", "content_required")
  end
  if is_present(message.tool_calls) then
    if message.role ~= "assistant" or not is_array(message.tool_calls) or #message.tool_calls == 0 then
      return failure("invalid_request", path .. ".tool_calls", "assistant_tool_calls_required")
    end
    for index, call in ipairs(message.tool_calls) do
      local call_path = path .. ".tool_calls[" .. index .. "]"
      if type(call) ~= "table" or call.type ~= "function" or type(call.id) ~= "string"
          or call.id == "" or type(call["function"]) ~= "table"
          or not function_name(call["function"].name) then
        return failure("invalid_request", call_path, "function_call_invalid")
      end
      unknown = reject_unknown(call, { id = true, type = true, ["function"] = true })
      if unknown then return unknown end
      unknown = reject_unknown(call["function"], { name = true, arguments = true })
      if unknown then return unknown end
      local arguments, err = parse_arguments(call["function"].arguments, call_path .. ".function.arguments")
      if err then return err end
      conversation[#conversation + 1] = item("assistant", nil, call.id, call["function"].name, arguments)
    end
  end
  return nil
end

reject_unknown = function(body, allowed)
  for key, value in pairs(body) do
    if not allowed[key] then
      return failure("unsupported_parameter", key, "field_not_supported")
    end
  end
  return nil
end

local function decode_tools(protocol, source)
  if not is_present(source) then return array({}) end
  if not is_array(source) then
    return nil, failure("invalid_request", "tools", "tools_array_required")
  end
  local tools = array({})
  local names = {}
  for index, raw in ipairs(source) do
    local path = "tools[" .. index .. "]"
    if type(raw) ~= "table" then
      return nil, failure("invalid_request", path, "function_definition_invalid")
    end
    local definition = raw
    if protocol == "chat_completions" then
      if raw.type ~= "function" or type(raw["function"]) ~= "table" then
        return nil, failure("unsupported_parameter", path .. ".type", "only_function_tools_supported")
      end
      local err = reject_unknown(raw, { type = true, ["function"] = true })
      if err then return nil, err end
      definition = raw["function"]
    elseif protocol == "responses" then
      if raw.type ~= "function" then
        return nil, failure("unsupported_parameter", path .. ".type", "only_function_tools_supported")
      end
    end
    local err = reject_unknown(definition, {
      type = protocol == "responses", name = true, description = true,
      parameters = protocol ~= "anthropic_messages",
      input_schema = protocol == "anthropic_messages", strict = protocol ~= "anthropic_messages",
    })
    if err then return nil, err end
    if not function_name(definition.name) or names[definition.name] then
      return nil, failure("invalid_request", path .. ".name", "function_name_invalid_or_duplicate")
    end
    names[definition.name] = true
    local schema = protocol == "anthropic_messages" and definition.input_schema or definition.parameters
    if type(schema) ~= "table" or is_array(schema) then
      return nil, failure("invalid_request", path .. ".parameters", "function_schema_object_required")
    end
    if is_present(definition.strict) and type(definition.strict) ~= "boolean" then
      return nil, failure("invalid_request", path .. ".strict", "strict_must_be_boolean")
    end
    tools[#tools + 1] = {
      name = definition.name,
      description = definition.description or aster.null,
      parameters = schema,
      strict = definition.strict == nil and aster.null or definition.strict,
    }
  end
  return tools
end

local function decode_tool_choice(protocol, source)
  if not is_present(source) then return nil end
  if type(source) == "string" then
    if protocol == "anthropic_messages" then
      return nil, failure("invalid_request", "tool_choice", "tool_choice_object_required")
    end
    if source == "auto" or source == "none" or source == "required" then
      return { mode = source, name = aster.null }
    end
  elseif type(source) == "table" then
    if protocol == "anthropic_messages" then
      local err = reject_unknown(source, { type = true, name = true, disable_parallel_tool_use = true })
      if err then return nil, err end
      local parallel = aster.null
      if is_present(source.disable_parallel_tool_use) then
        if type(source.disable_parallel_tool_use) ~= "boolean" then
          return nil, failure("invalid_request", "tool_choice.disable_parallel_tool_use", "boolean_required")
        end
        parallel = not source.disable_parallel_tool_use
      end
      local mode = source.type == "any" and "required" or source.type
      if mode == "tool" and function_name(source.name) then
        return { mode = "named", name = source.name, parallel = parallel }
      end
      if mode == "auto" or mode == "none" or mode == "required" then
        return { mode = mode, name = aster.null, parallel = parallel }
      end
    elseif protocol == "chat_completions" and source.type == "function"
        and type(source["function"]) == "table" and function_name(source["function"].name) then
      local err = reject_unknown(source, { type = true, ["function"] = true })
      if err then return nil, err end
      err = reject_unknown(source["function"], { name = true })
      if err then return nil, err end
      return { mode = "named", name = source["function"].name }
    elseif protocol == "responses" and source.type == "function" and function_name(source.name) then
      local err = reject_unknown(source, { type = true, name = true })
      if err then return nil, err end
      return { mode = "named", name = source.name }
    end
  end
  return nil, failure("unsupported_parameter", "tool_choice", "tool_choice_not_supported")
end

function M.describe()
  return ok({
    host_api = 1,
    canonical_schema = 2,
    rule_revision = public_rules.revision,
    channels = channels,
  })
end

function M.decode_request(input)
  if type(input) ~= "table" or type(input.body) ~= "table" then
    return failure("invalid_request", aster.null, "request_body_required")
  end
  local body = input.body
  local protocol = input.protocol
  if type(body.model) ~= "string" or body.model == "" then
    return failure("invalid_request", "model", "model_required")
  end
  if is_present(body.stream) and type(body.stream) ~= "boolean" then
    return failure("invalid_request", "stream", "stream_must_be_boolean")
  end
  local conversation = array({})
  local parameters = {}
  local source_protocol
  local tools, tools_error = decode_tools(protocol, body.tools)
  if tools_error then return tools_error end
  if protocol == "chat_completions" then
    source_protocol = "chat_completions"
    local err = reject_unknown(body, {
      model = true, messages = true, stream = true, temperature = true,
      top_p = true, max_tokens = true, max_completion_tokens = true,
      reasoning_effort = true, tools = true, tool_choice = true,
      parallel_tool_calls = true, response_format = true,
    })
    if err then return err end
    if not is_array(body.messages) or #body.messages == 0 then
      return failure("invalid_request", "messages", "messages_required")
    end
    for index, message in ipairs(body.messages) do
      err = append_message(conversation, message, "messages[" .. index .. "]")
      if err then return err end
    end
    if is_present(body.max_tokens) and is_present(body.max_completion_tokens) then
      return failure("unsupported_parameter", "max_tokens", "conflicting_token_limits")
    end
    if is_present(body.max_completion_tokens) then
      parameters.max_output_tokens = body.max_completion_tokens
    elseif is_present(body.max_tokens) then
      parameters.max_output_tokens = body.max_tokens
    end
    parameters.reasoning_effort = body.reasoning_effort
    parameters.parallel_tool_calls = body.parallel_tool_calls
    parameters.response_format = body.response_format
  elseif protocol == "responses" then
    source_protocol = "responses"
    local err = reject_unknown(body, {
      model = true, input = true, instructions = true, stream = true,
      temperature = true, top_p = true, max_output_tokens = true,
      reasoning = true, store = true, tools = true, tool_choice = true,
      parallel_tool_calls = true, text = true,
    })
    if err then return err end
    if body.store == true then
      return failure("unsupported_parameter", "store", "stateful_response_not_supported")
    end
    if is_present(body.instructions) then
      if type(body.instructions) ~= "string" then
        return failure("invalid_request", "instructions", "instructions_must_be_text")
      end
      conversation[#conversation + 1] = item("system", array({ { kind = "text", text = body.instructions } }))
    end
    if type(body.input) == "string" then
      conversation[#conversation + 1] = item("user", array({ { kind = "text", text = body.input } }))
    elseif is_array(body.input) then
      if #body.input == 0 then
        return failure("invalid_request", "input", "input_required")
      end
      for index, message in ipairs(body.input) do
        local path = "input[" .. index .. "]"
        if type(message) == "table" and message.type == "function_call" then
          err = reject_unknown(message, { type = true, call_id = true, name = true, arguments = true })
          if err then return err end
          if type(message.call_id) ~= "string" or message.call_id == ""
              or not function_name(message.name) then
            return failure("invalid_request", path, "function_call_invalid")
          end
          local args, parse_error = parse_arguments(message.arguments, path .. ".arguments")
          if parse_error then return parse_error end
          conversation[#conversation + 1] = item("assistant", nil, message.call_id, message.name, args)
        elseif type(message) == "table" and message.type == "function_call_output" then
          err = reject_unknown(message, { type = true, call_id = true, output = true })
          if err then return err end
          if type(message.call_id) ~= "string" or message.call_id == "" then
            return failure("invalid_request", path .. ".call_id", "tool_call_id_required")
          end
          local content, content_error = text_content(message.output, path .. ".output")
          if content_error then return content_error end
          conversation[#conversation + 1] = item("tool", content, message.call_id)
        else
          err = append_message(conversation, message, path)
          if err then return err end
        end
      end
    else
      return failure("invalid_request", "input", "input_required")
    end
    parameters.max_output_tokens = body.max_output_tokens
    if is_present(body.reasoning) then
      if type(body.reasoning) ~= "table" then
        return failure("invalid_request", "reasoning", "reasoning_invalid")
      end
      err = reject_unknown(body.reasoning, { effort = true, summary = true })
      if err then return err end
      if is_present(body.reasoning.summary) then
        return failure("unsupported_parameter", "reasoning.summary", "reasoning_summary_not_supported")
      end
      parameters.reasoning_effort = body.reasoning.effort
    end
    parameters.parallel_tool_calls = body.parallel_tool_calls
    if is_present(body.text) then
      if type(body.text) ~= "table" then
        return failure("invalid_request", "text", "text_format_invalid")
      end
      local err = reject_unknown(body.text, { format = true })
      if err then return err end
      parameters.response_format = body.text.format
    end
  elseif protocol == "anthropic_messages" then
    source_protocol = "anthropic_messages"
    local err = reject_unknown(body, {
      model = true, system = true, messages = true, stream = true,
      max_tokens = true, temperature = true, top_p = true,
      tools = true, tool_choice = true, thinking = true,
    })
    if err then return err end
    if type(body.max_tokens) ~= "number" then
      return failure("invalid_request", "max_tokens", "max_tokens_required")
    end
    if is_present(body.system) then
      if type(body.system) ~= "string" then
        return failure("unsupported_parameter", "system", "system_blocks_not_supported")
      end
      conversation[#conversation + 1] = item("system", array({ { kind = "text", text = body.system } }))
    end
    if not is_array(body.messages) or #body.messages == 0 then
      return failure("invalid_request", "messages", "messages_required")
    end
    for index, message in ipairs(body.messages) do
      local path = "messages[" .. index .. "]"
      if type(message) ~= "table" or (message.role ~= "user" and message.role ~= "assistant") then
        return failure("invalid_request", path .. ".role", "message_role_invalid")
      end
      err = reject_unknown(message, { role = true, content = true })
      if err then return err end
      if type(message.content) == "string" then
        conversation[#conversation + 1] = item(message.role, array({ { kind = "text", text = message.content } }))
      elseif is_array(message.content) then
        for block_index, block in ipairs(message.content) do
          local block_path = path .. ".content[" .. block_index .. "]"
          if type(block) ~= "table" then
            return failure("invalid_request", block_path, "content_block_invalid")
          end
          if block.type == "text" then
            err = reject_unknown(block, { type = true, text = true })
            if err then return err end
            if type(block.text) ~= "string" then
              return failure("invalid_request", block_path .. ".text", "text_required")
            end
            conversation[#conversation + 1] = item(message.role, array({ { kind = "text", text = block.text } }))
          elseif block.type == "tool_use" and message.role == "assistant" then
            err = reject_unknown(block, { type = true, id = true, name = true, input = true })
            if err then return err end
            if type(block.id) ~= "string" or block.id == "" or not function_name(block.name) then
              return failure("invalid_request", block_path, "function_call_invalid")
            end
            local args, parse_error = parse_arguments(block.input, block_path .. ".input")
            if parse_error then return parse_error end
            conversation[#conversation + 1] = item("assistant", nil, block.id, block.name, args)
          elseif block.type == "tool_result" and message.role == "user" then
            err = reject_unknown(block, { type = true, tool_use_id = true, content = true })
            if err then return err end
            if type(block.tool_use_id) ~= "string" or block.tool_use_id == "" then
              return failure("invalid_request", block_path .. ".tool_use_id", "tool_call_id_required")
            end
            local content, content_error = text_content(block.content, block_path .. ".content")
            if content_error then return content_error end
            conversation[#conversation + 1] = item("tool", content, block.tool_use_id)
          else
            return failure("unsupported_parameter", block_path .. ".type", "content_block_not_supported")
          end
        end
      else
        return failure("invalid_request", path .. ".content", "content_required")
      end
    end
    parameters.max_output_tokens = body.max_tokens
    if is_present(body.thinking) then
      if type(body.thinking) ~= "table" then
        return failure("invalid_request", "thinking", "thinking_invalid")
      end
      err = reject_unknown(body.thinking, { type = true, budget_tokens = true })
      if err then return err end
      if body.thinking.type ~= "disabled" and body.thinking.type ~= "enabled" then
        return failure("unsupported_parameter", "thinking.type", "thinking_mode_not_supported")
      end
      parameters.thinking_mode = body.thinking.type
      parameters.thinking_budget_tokens = body.thinking.budget_tokens
    end
  else
    return failure("unsupported_operation", "protocol", "protocol_not_supported")
  end
  local choice, choice_error = decode_tool_choice(protocol, body.tool_choice)
  if choice_error then return choice_error end
  if choice then
    parameters.tool_choice = choice
    if is_present(choice.parallel) then
      parameters.parallel_tool_calls = choice.parallel
    end
  end
  if is_present(parameters.parallel_tool_calls) and type(parameters.parallel_tool_calls) ~= "boolean" then
    return failure("invalid_request", "parallel_tool_calls", "boolean_required")
  end
  if choice and choice.mode ~= "none" and #tools == 0 then
    return failure("invalid_request", "tool_choice", "tools_required")
  end
  parameters.temperature = body.temperature
  parameters.top_p = body.top_p
  return ok({
    schema_version = 2,
    kind = "generate",
    public_model = body.model,
    source_protocol = source_protocol,
    stream = body.stream == true,
    conversation = conversation,
    tools = tools,
    parameters = parameters,
    assets = array({}),
    provenance = array({}),
  })
end

local function rejection(path, reason)
  return {
    code = "unsupported_parameter",
    reason_key = reason,
    source_path = path,
    allowed_values = aster.null,
  }
end

local function known_glm_reasoning_model(name)
  local rule = public_rules.models[name]
  return rule and rule.provider == "glm" and rule.reasoning ~= nil
end

local function mapping(changes, path, target, requested, effective, rule_id)
  changes[#changes + 1] = {
    source_path = path,
    target_path = target,
    kind = requested == effective and "lossless" or "mapped",
    requested = requested,
    effective = effective,
    rule_id = rule_id,
  }
end

local function mapped_value(plan, path, requested)
  for _, change in ipairs(plan.changes) do
    if change.source_path == path then return change.effective end
  end
  return requested
end

local function tool_history_requirement(conversation)
  local pending = {}
  local used = {}
  local required = false
  for index, item_value in ipairs(conversation) do
    if is_present(item_value.tool_name) then
      required = true
      local id = item_value.call_id
      if type(id) ~= "string" or id == "" or used[id] then
        return nil, rejection("conversation[" .. index .. "].call_id", "tool_call_id_duplicate")
      end
      pending[id] = true
      used[id] = true
    elseif item_value.role == "tool" then
      required = true
      local id = item_value.call_id
      if type(id) ~= "string" or not pending[id] then
        return nil, rejection("conversation[" .. index .. "].call_id", "tool_result_without_call")
      end
      pending[id] = nil
    end
  end
  if next(pending) ~= nil then
    return nil, rejection("conversation", "tool_call_missing_result")
  end
  return required
end

function M.assess(input)
  if type(input) ~= "table" or type(input.operation) ~= "table"
      or type(input.target) ~= "table" then
    return failure("invalid_request", aster.null, "assessment_invalid")
  end
  local channel = channels[input.target.channel_id]
  if not channel then
    return failure("unsupported_channel", "target.channel_id", "channel_not_supported")
  end
  local op = input.operation
  local p = op.parameters or {}
  local wire_protocol = channel.wire_protocol
  if (channel.provider == "openai" or channel.provider == "deepseek")
      and op.source_protocol == "chat_completions" then
    wire_protocol = "chat"
  elseif (channel.provider == "openai" or channel.provider == "deepseek")
      and op.source_protocol == "responses" then
    wire_protocol = "responses"
  elseif (channel.provider == "openai" or channel.provider == "deepseek")
      and op.source_protocol == "anthropic_messages" then
    wire_protocol = "chat"
  end
  local changes = array({})
  local rejected = array({})
  local history_tools, history_error = tool_history_requirement(op.conversation)
  if history_error then rejected[#rejected + 1] = history_error end
  if channel.provider == "deepseek" and op.source_protocol == "chat_completions"
      and history_tools then
    wire_protocol = "responses"
  end
  local mode = input.compatibility_mode
  if mode ~= "compatible" and mode ~= "strict" then
    return failure("invalid_compatibility_mode", "compatibility_mode", "compatibility_mode_invalid")
  end
  if op.schema_version ~= 2 or op.kind ~= "generate" then
    return failure("unsupported_operation", "operation.kind", "operation_not_supported")
  end
  local unknown = reject_unknown(p, {
    temperature = true, top_p = true, max_output_tokens = true,
    reasoning_effort = true, tool_choice = true,
    parallel_tool_calls = true, response_format = true,
    thinking_mode = true, thinking_budget_tokens = true,
  })
  if unknown then return unknown end
  local model = input.target.upstream_model
  local model_rule = public_rules.models[model]
  local deepseek_thinking = channel.provider == "deepseek"
      and model_rule ~= nil and model_rule.provider == "deepseek"
      and p.reasoning_effort ~= "none"
  for _, field in ipairs({ "temperature", "top_p" }) do
    local value = p[field]
    if is_present(value) then
      if type(value) ~= "number" or value < 0 or value > (field == "temperature" and 2 or 1) then
        rejected[#rejected + 1] = rejection("parameters." .. field, "sampling_value_invalid")
      elseif channel.provider == "deepseek" and model_rule then
        if field == "temperature" and deepseek_thinking then
          rejected[#rejected + 1] = rejection("parameters.temperature", "sampling_ignored_during_thinking")
        elseif field == "top_p" and not deepseek_thinking then
          rejected[#rejected + 1] = rejection("parameters.top_p", "sampling_ignored_without_thinking")
        elseif field == "top_p" and value < 0.95 then
          if mode == "compatible" then
            mapping(changes, "parameters.top_p", "top_p", value, 0.95,
              "deepseek.top_p.thinking_floor.v1")
          else
            rejected[#rejected + 1] = rejection("parameters.top_p", "top_p_thinking_floor")
          end
        end
      elseif channel.provider == "glm" and (not known_glm_reasoning_model(model)
          or (is_present(p.thinking_mode) and p.thinking_mode ~= "disabled")) then
        rejected[#rejected + 1] = rejection("parameters." .. field, "sampling_model_rule_missing")
      end
    end
  end
  if is_present(p.reasoning_effort) then
    if channel.provider == "openai"
        and (model:match("^gpt%-5") or model:match("^o[134]")) then
      local allowed = { default = true, minimal = true, low = true,
        medium = true, high = true, xhigh = true }
      if not allowed[p.reasoning_effort] then
        rejected[#rejected + 1] = rejection("parameters.reasoning_effort", "reasoning_effort_not_supported")
      end
    elseif model_rule and model_rule.provider == channel.provider and model_rule.reasoning then
      local reason_rule = model_rule.reasoning
      local mapped = reason_rule.mapped[p.reasoning_effort]
      local native = false
      for _, value in ipairs(reason_rule.native) do
        if value == p.reasoning_effort then native = true end
      end
      if mapped and mode == "compatible" then
        mapping(changes, "parameters.reasoning_effort",
          wire_protocol == "chat" and "reasoning_effort" or "reasoning.effort",
          p.reasoning_effort, mapped, model_rule.provider .. ".reasoning.approximate.v1")
      elseif not native then
        rejected[#rejected + 1] = rejection("parameters.reasoning_effort", "reasoning_effort_not_supported")
      end
    else
      rejected[#rejected + 1] = rejection("parameters.reasoning_effort", "reasoning_model_rule_missing")
    end
  end
  if is_present(p.thinking_mode) or is_present(p.thinking_budget_tokens) then
    if channel.provider ~= "glm" or not known_glm_reasoning_model(model)
        or is_present(p.thinking_budget_tokens)
        or (p.thinking_mode ~= "enabled" and p.thinking_mode ~= "disabled") then
      rejected[#rejected + 1] = rejection("parameters.thinking_mode", "thinking_budget_model_rule_missing")
    end
  end
  if p.thinking_mode == "disabled" and is_present(p.reasoning_effort) then
    rejected[#rejected + 1] = rejection("parameters.reasoning_effort", "reasoning_disabled_conflict")
  end
  if op.stream and op.source_protocol == "anthropic_messages"
      and (p.thinking_mode == "enabled" or is_present(p.reasoning_effort)
        or deepseek_thinking) then
    rejected[#rejected + 1] = rejection("parameters.thinking_mode", "thinking_stream_not_portable")
  end
  if is_present(p.response_format) then
    local format = p.response_format
    if type(format) ~= "table" or (format.type ~= "text" and format.type ~= "json_object") then
      rejected[#rejected + 1] = rejection("parameters.response_format", "structured_output_model_rule_missing")
    elseif channel.provider == "glm" and not known_glm_reasoning_model(model) then
      rejected[#rejected + 1] = rejection("parameters.response_format", "structured_output_model_rule_missing")
    end
  end
  for index, tool in ipairs(op.tools) do
    if tool.strict == true then
      rejected[#rejected + 1] = rejection("tools[" .. index .. "].strict", "strict_tool_schema_rule_missing")
    end
  end
  if is_present(p.parallel_tool_calls) and channel.provider ~= "openai" then
    if p.parallel_tool_calls == false and #op.tools > 0 then
      rejected[#rejected + 1] = rejection("parameters.parallel_tool_calls", "parallel_tool_constraint_not_preserved")
    elseif channel.provider == "glm" and #op.tools > 0 then
      rejected[#rejected + 1] = rejection("parameters.parallel_tool_calls", "parallel_tool_rule_missing")
    end
  end
  if is_present(p.tool_choice) then
    local choice = p.tool_choice
    if type(choice) ~= "table" or type(choice.mode) ~= "string" then
      rejected[#rejected + 1] = rejection("parameters.tool_choice", "tool_choice_invalid")
    elseif choice.mode == "named" then
      local found = false
      for _, tool in ipairs(op.tools) do
        if tool.name == choice.name then found = true end
      end
      if not found then
        rejected[#rejected + 1] = rejection("parameters.tool_choice.name", "named_tool_not_found")
      end
    end
    if type(choice) == "table" and channel.provider ~= "openai" and choice.mode ~= "auto"
        and not (choice.mode == "none" and #op.tools == 0) then
      rejected[#rejected + 1] = rejection("parameters.tool_choice", "tool_choice_model_rule_missing")
    end
  end
  if is_present(p.max_output_tokens) then
    if type(p.max_output_tokens) ~= "number" or p.max_output_tokens < 1
        or p.max_output_tokens % 1 ~= 0 then
      rejected[#rejected + 1] = rejection("parameters.max_output_tokens", "token_limit_invalid")
    elseif wire_protocol == "chat" then
      local token_field = channel.provider == "openai" and "max_completion_tokens" or "max_tokens"
      changes[#changes + 1] = {
        source_path = "parameters.max_output_tokens",
        target_path = token_field,
        kind = "lossless",
        requested = p.max_output_tokens,
        effective = p.max_output_tokens,
        rule_id = "chat.max_tokens.v1",
      }
    end
  end
  return ok({
    schema = 1,
    compatible = #rejected == 0,
    rule_revision = public_rules.revision,
    target = {
      connection_id = input.target.connection_id,
      connection_revision = input.target.connection_revision,
      upstream_model = input.target.upstream_model,
      wire_protocol = wire_protocol,
    },
    required_features = (#op.tools > 0 or history_tools) and array({ "text", "tools" }) or array({ "text" }),
    changes = changes,
    rejected = rejected,
  })
end

local function conversation_to_chat(items)
  local messages = array({})
  for _, item in ipairs(items) do
    if is_present(item.tool_name) then
      if item.role ~= "assistant" or not is_present(item.call_id) or not is_present(item.arguments) then
        return nil
      end
      messages[#messages + 1] = {
        role = "assistant", content = aster.null,
        tool_calls = array({ {
          id = item.call_id, type = "function",
          ["function"] = { name = item.tool_name, arguments = aster.stringify_json(item.arguments) },
        } }),
      }
    elseif item.role == "tool" then
      if not is_present(item.call_id) or #item.content ~= 1 or item.content[1].kind ~= "text" then
        return nil
      end
      messages[#messages + 1] = {
        role = "tool", tool_call_id = item.call_id, content = item.content[1].text,
      }
    else
      if #item.content ~= 1 or item.content[1].kind ~= "text" then
        return nil
      end
      messages[#messages + 1] = { role = item.role, content = item.content[1].text }
    end
  end
  return messages
end

local function conversation_to_responses(items)
  local input = array({})
  for _, item in ipairs(items) do
    if is_present(item.tool_name) then
      if item.role ~= "assistant" or not is_present(item.call_id) or not is_present(item.arguments) then
        return nil
      end
      input[#input + 1] = {
        type = "function_call", call_id = item.call_id, name = item.tool_name,
        arguments = aster.stringify_json(item.arguments),
      }
    elseif item.role == "tool" then
      if not is_present(item.call_id) or #item.content ~= 1 or item.content[1].kind ~= "text" then
        return nil
      end
      input[#input + 1] = {
        type = "function_call_output", call_id = item.call_id, output = item.content[1].text,
      }
    else
      if #item.content ~= 1 or item.content[1].kind ~= "text" then
        return nil
      end
      input[#input + 1] = {
        role = item.role,
        content = array({ { type = "input_text", text = item.content[1].text } }),
      }
    end
  end
  return input
end

local function tools_to_chat(tools)
  local result = array({})
  for _, tool in ipairs(tools) do
    local definition = { name = tool.name, parameters = tool.parameters }
    if is_present(tool.description) then definition.description = tool.description end
    if is_present(tool.strict) then definition.strict = tool.strict end
    result[#result + 1] = {
      type = "function",
      ["function"] = definition,
    }
  end
  return result
end

local function tools_to_responses(tools)
  local result = array({})
  for _, tool in ipairs(tools) do
    local definition = { type = "function", name = tool.name, parameters = tool.parameters }
    if is_present(tool.description) then definition.description = tool.description end
    if is_present(tool.strict) then definition.strict = tool.strict end
    result[#result + 1] = definition
  end
  return result
end

local function encode_tool_choice(choice, wire_protocol)
  if not is_present(choice) then return nil end
  if choice.mode ~= "named" then return choice.mode end
  if wire_protocol == "responses" then
    return { type = "function", name = choice.name }
  end
  return { type = "function", ["function"] = { name = choice.name } }
end

function M.prepare(input)
  if type(input) ~= "table" or type(input.operation) ~= "table"
      or type(input.plan) ~= "table" or type(input.target) ~= "table" then
    return failure("invalid_request", aster.null, "preparation_invalid")
  end
  local channel = channels[input.target.channel_id]
  if not channel or input.plan.compatible ~= true
      or input.plan.target.upstream_model ~= input.target.upstream_model
      or (input.plan.target.wire_protocol ~= channel.wire_protocol
          and not ((channel.provider == "openai" or channel.provider == "deepseek")
            and (input.plan.target.wire_protocol == "chat"
              or input.plan.target.wire_protocol == "responses"))) then
    return failure("unsupported_channel", "target", "plan_target_mismatch")
  end
  local messages
  if input.plan.target.wire_protocol == "responses" then
    messages = conversation_to_responses(input.operation.conversation)
  else
    messages = conversation_to_chat(input.operation.conversation)
  end
  if not messages then
    return failure("unsupported_parameter", "conversation", "content_part_not_supported")
  end
  local parameters = input.operation.parameters or {}
  local body = { model = input.target.upstream_model, stream = input.operation.stream == true }
  local path
  if input.plan.target.wire_protocol == "responses" then
    body.input = messages
    body.max_output_tokens = parameters.max_output_tokens
    if #input.operation.tools > 0 then body.tools = tools_to_responses(input.operation.tools) end
    if is_present(parameters.reasoning_effort) then
      body.reasoning = { effort = mapped_value(input.plan,
        "parameters.reasoning_effort", parameters.reasoning_effort) }
    end
    if is_present(parameters.response_format) then
      body.text = { format = parameters.response_format }
    end
    path = "/responses"
  else
    body.messages = messages
    if channel.provider == "openai" then
      body.max_completion_tokens = parameters.max_output_tokens
    else
      body.max_tokens = parameters.max_output_tokens
    end
    if body.stream and channel.provider == "openai" then
      body.stream_options = { include_usage = true }
    end
    if #input.operation.tools > 0 then body.tools = tools_to_chat(input.operation.tools) end
    if channel.provider == "glm" and (is_present(parameters.reasoning_effort)
        or is_present(parameters.thinking_mode)) then
      body.thinking = { type = parameters.thinking_mode or "enabled" }
    end
    if is_present(parameters.reasoning_effort) then
      body.reasoning_effort = mapped_value(input.plan,
        "parameters.reasoning_effort", parameters.reasoning_effort)
    end
    body.response_format = parameters.response_format
    path = "/chat/completions"
  end
  body.temperature = parameters.temperature
  body.top_p = is_present(parameters.top_p) and mapped_value(input.plan,
    "parameters.top_p", parameters.top_p) or nil
  body.tool_choice = encode_tool_choice(parameters.tool_choice, input.plan.target.wire_protocol)
  if channel.provider == "openai" then
    body.parallel_tool_calls = parameters.parallel_tool_calls
  end
  return ok({
    action = "execute",
    method = "POST",
    endpoint_id = channel.profile,
    relative_path = path,
    public_headers = array({ { "content-type", "application/json" } }),
    secret_bindings = array({ { slot = "api_key", destination = "authorization_bearer" } }),
    body = body,
    response_mode = input.operation.stream and "sse" or "json",
    timeout_ms = 120000,
    redirect_policy = "deny",
  })
end

local function usage_value(raw, input_key, cache_key, output_key, reasoning_key)
  if not is_present(raw) then return aster.null end
  if type(raw) ~= "table" then return nil end
  local input_tokens = raw[input_key]
  local output_tokens = raw[output_key]
  local cached = raw[cache_key]
  local reasoning = raw[reasoning_key]
  if input_key == "prompt_tokens" and type(raw.prompt_tokens_details) == "table" then
    cached = raw.prompt_tokens_details.cached_tokens
  elseif input_key == "input_tokens" and type(raw.input_tokens_details) == "table" then
    cached = raw.input_tokens_details.cached_tokens
  end
  if output_key == "completion_tokens" and type(raw.completion_tokens_details) == "table" then
    reasoning = raw.completion_tokens_details.reasoning_tokens
  elseif output_key == "output_tokens" and type(raw.output_tokens_details) == "table" then
    reasoning = raw.output_tokens_details.reasoning_tokens
  end
  local function valid_count(value)
    return not is_present(value) or (type(value) == "number" and value >= 0 and value % 1 == 0)
  end
  if not valid_count(input_tokens) or not valid_count(output_tokens)
      or not valid_count(cached) or not valid_count(reasoning) then
    return nil
  end
  return {
    input_tokens = input_tokens or aster.null,
    cached_input_tokens = cached or aster.null,
    output_tokens = output_tokens or aster.null,
    reasoning_output_tokens = reasoning or aster.null,
    raw = raw,
  }
end

local function parsed_result(model, id, reason, output, usage)
  return ok({
    schema_version = 2,
    public_model = model,
    upstream_id = id or aster.null,
    finish_reason = reason,
    output = output,
    usage = usage,
  })
end

local function append_upstream_tool(output, call_id, name, arguments, path)
  if type(call_id) ~= "string" or call_id == "" or not function_name(name) then
    return failure("upstream_contract_violation", path, "function_call_invalid")
  end
  local parsed, err = parse_arguments(arguments, path .. ".arguments")
  if err then return failure("upstream_contract_violation", path .. ".arguments", "function_arguments_invalid") end
  output[#output + 1] = { kind = "tool_call", call_id = call_id, name = name, arguments = parsed }
  return nil
end

function M.parse_buffered(input)
  if type(input) ~= "table" or type(input.body) ~= "table"
      or type(input.public_model) ~= "string" then
    return failure("upstream_contract_violation", aster.null, "upstream_body_invalid")
  end
  local body = input.body
  local output = array({})
  local usage
  local reason
  if input.wire_protocol == "chat" then
    if not is_array(body.choices) or #body.choices ~= 1 then
      return failure("upstream_contract_violation", "choices", "one_choice_required")
    end
    local choice = body.choices[1]
    if type(choice) ~= "table" then
      return failure("upstream_contract_violation", "choices[1]", "choice_invalid")
    end
    local message = choice.message
    if type(message) ~= "table" or message.role ~= "assistant" then
      return failure("upstream_contract_violation", "choices[1].message", "assistant_message_required")
    end
    if is_present(message.content) then
      if type(message.content) ~= "string" then
        return failure("upstream_contract_violation", "choices[1].message.content", "text_required")
      end
      output[#output + 1] = { kind = "text", text = message.content }
    end
    if is_present(message.reasoning_content) then
      if type(message.reasoning_content) ~= "string" then
        return failure("upstream_contract_violation", "choices[1].message.reasoning_content", "reasoning_text_invalid")
      end
      output[#output + 1] = { kind = "reasoning_text", text = message.reasoning_content }
    end
    if is_present(message.tool_calls) then
      if not is_array(message.tool_calls) then
        return failure("upstream_contract_violation", "choices[1].message.tool_calls", "tool_calls_invalid")
      end
      for index, tool in ipairs(message.tool_calls) do
        if type(tool) ~= "table" or tool.type ~= "function" or type(tool["function"]) ~= "table" then
          return failure("upstream_contract_violation", "choices[1].message.tool_calls", "function_call_invalid")
        end
        local err = append_upstream_tool(output, tool.id, tool["function"].name,
          tool["function"].arguments, "choices[1].message.tool_calls[" .. index .. "]")
        if err then return err end
      end
    end
    local reasons = { stop = "stop", tool_calls = "tool_calls",
      length = "length", content_filter = "content_filter" }
    reason = reasons[choice.finish_reason]
    if choice.finish_reason == "insufficient_system_resource" then
      return failure("upstream_overloaded", "choices[1].finish_reason", "upstream_resource_exhausted")
    elseif choice.finish_reason == "aborted" then
      return failure("upstream_failed", "choices[1].finish_reason", "upstream_generation_aborted")
    end
    usage = usage_value(body.usage, "prompt_tokens", "cached_tokens", "completion_tokens", "reasoning_tokens")
  elseif input.wire_protocol == "responses" then
    if body.status == "failed" then
      return failure("upstream_failed", "status", "upstream_response_failed")
    end
    if body.status ~= "completed" and body.status ~= "incomplete" then
      return failure("upstream_contract_violation", "status", "response_not_complete")
    end
    if not is_array(body.output) then
      return failure("upstream_contract_violation", "output", "output_array_required")
    end
    for index, item_value in ipairs(body.output) do
      if type(item_value) ~= "table" then
        return failure("upstream_contract_violation", "output[" .. index .. "]", "output_item_invalid")
      end
      if item_value.type == "message" then
        if not is_array(item_value.content) then
          return failure("upstream_contract_violation", "output", "content_array_required")
        end
        for _, part in ipairs(item_value.content) do
          if part.type == "output_text" and type(part.text) == "string" then
            output[#output + 1] = { kind = "text", text = part.text }
          elseif part.type == "refusal" then
            reason = "content_filter"
          else
            return failure("upstream_contract_violation", "output", "output_part_unsupported")
          end
        end
      elseif item_value.type == "function_call" then
        local err = append_upstream_tool(output, item_value.call_id, item_value.name,
          item_value.arguments, "output[" .. index .. "]")
        if err then return err end
      elseif item_value.type == "reasoning" then
        if is_array(item_value.summary) then
          for _, part in ipairs(item_value.summary) do
            if part.type == "summary_text" and type(part.text) == "string" then
              output[#output + 1] = { kind = "reasoning_text", text = part.text }
            end
          end
        end
      else
        return failure("upstream_contract_violation", "output", "output_item_unsupported")
      end
    end
    if body.status == "incomplete" then
      reason = type(body.incomplete_details) == "table"
          and body.incomplete_details.reason == "content_filter" and "content_filter" or "length"
    end
    usage = usage_value(body.usage, "input_tokens", "cached_tokens", "output_tokens", "reasoning_tokens")
  elseif input.wire_protocol == "anthropic" then
    if not is_array(body.content) then
      return failure("upstream_contract_violation", "content", "content_array_required")
    end
    for index, block in ipairs(body.content) do
      if type(block) ~= "table" then
        return failure("upstream_contract_violation", "content[" .. index .. "]", "content_block_invalid")
      end
      if block.type == "text" and type(block.text) == "string" then
        output[#output + 1] = { kind = "text", text = block.text }
      elseif block.type == "tool_use" then
        local err = append_upstream_tool(output, block.id, block.name, block.input, "content[" .. index .. "]")
        if err then return err end
      else
        return failure("upstream_contract_violation", "content", "content_block_unsupported")
      end
    end
    local reasons = { end_turn = "stop", tool_use = "tool_calls", max_tokens = "length",
      stop_sequence = "stop", refusal = "content_filter" }
    reason = reasons[body.stop_reason]
    usage = usage_value(body.usage, "input_tokens", "cache_read_input_tokens", "output_tokens", "reasoning_tokens")
    if type(usage) == "table" then
      local raw = body.usage
      if is_present(raw.cache_creation_input_tokens) then
        usage.raw.cache_creation_input_tokens = raw.cache_creation_input_tokens
      end
    end
  else
    return failure("unsupported_channel", "wire_protocol", "wire_protocol_not_supported")
  end
  if not reason and input.wire_protocol ~= "responses" then
    return failure("upstream_contract_violation", "finish_reason", "finish_reason_unknown")
  end
  if not reason then
    local has_tool = false
    for _, item_value in ipairs(output) do
      if item_value.kind == "tool_call" then has_tool = true end
    end
    reason = has_tool and "tool_calls" or "stop"
  end
  if usage == nil then
    return failure("upstream_contract_violation", "usage", "usage_invalid")
  end
  return parsed_result(input.public_model, body.id, reason, output, usage)
end

-- The host forwards each verified SSE event as it arrives and retains the
-- terminal event until durable quota settlement. Chat providers report usage
-- in their final JSON chunk; no entire stream is passed into Lua.
function M.parse_stream_usage(input)
  if type(input) ~= "table" or input.wire_protocol ~= "chat"
      or type(input.usage) ~= "table" then
    return failure("upstream_contract_violation", "usage", "stream_usage_missing")
  end
  local usage = usage_value(input.usage, "prompt_tokens", "cached_tokens",
    "completion_tokens", "reasoning_tokens")
  if type(usage) ~= "table" or not is_present(usage.input_tokens)
      or not is_present(usage.output_tokens) then
    return failure("upstream_contract_violation", "usage", "stream_usage_invalid")
  end
  return ok(usage)
end

local public_usage

local function stream_event(events, kind, fields)
  fields.type = kind
  events[#events + 1] = fields
end

-- Convert a Chat SSE stream to the public Responses event sequence. State is
-- explicit JSON so a hot-loaded bundle snapshot remains fixed for the entire
-- request without mutable module globals.
function M.map_chat_to_responses_stream(input)
  if type(input) ~= "table" or type(input.public_id) ~= "string"
      or type(input.public_model) ~= "string" then
    return failure("invalid_request", aster.null, "stream_mapping_invalid")
  end
  local state = input.state
  if not is_present(state) then
    state = {
      id = input.public_id, model = input.public_model, created = input.created or 0,
      text = "", reasoning = "", text_index = aster.null,
      reasoning_index = aster.null, output_count = 0,
      tools = array({}), finish_reason = aster.null,
    }
  end
  if state.id ~= input.public_id or state.model ~= input.public_model then
    return failure("invalid_request", "state", "stream_state_mismatch")
  end
  local events = array({})
  if not state.started then
    stream_event(events, "response.created", {
      response = { id = state.id, object = "response", created_at = state.created,
        model = state.model, status = "in_progress", output = array({}), usage = aster.null },
    })
    state.started = true
  end
  if input.done == true then
    if not is_present(state.finish_reason) or not is_present(input.usage) then
      return failure("upstream_contract_violation", "finish_reason", "stream_terminal_incomplete")
    end
    local usage = usage_value(input.usage, "prompt_tokens", "cached_tokens",
      "completion_tokens", "reasoning_tokens")
    if type(usage) ~= "table" or not is_present(usage.input_tokens)
        or not is_present(usage.output_tokens) then
      return failure("upstream_contract_violation", "usage", "stream_usage_invalid")
    end
    local output = array({})
    if is_present(state.reasoning_index) then
      stream_event(events, "response.reasoning_text.done", {
        item_id = state.id .. "-reasoning", output_index = state.reasoning_index,
        content_index = 0, text = state.reasoning,
      })
      local item_value = {
        id = state.id .. "-reasoning", type = "reasoning", status = "completed",
        summary = array({ { type = "summary_text", text = state.reasoning } }),
      }
      stream_event(events, "response.output_item.done", {
        item = item_value, output_index = state.reasoning_index,
      })
      output[state.reasoning_index + 1] = item_value
    end
    if is_present(state.text_index) then
      stream_event(events, "response.output_text.done", {
        item_id = state.id .. "-message", output_index = state.text_index,
        content_index = 0, text = state.text,
      })
      local part = { type = "output_text", text = state.text, annotations = array({}) }
      stream_event(events, "response.content_part.done", {
        item_id = state.id .. "-message", output_index = state.text_index,
        content_index = 0, part = part,
      })
      local item_value = {
        id = state.id .. "-message", type = "message", status = "completed",
        role = "assistant", content = array({ part }),
      }
      stream_event(events, "response.output_item.done", {
        item = item_value, output_index = state.text_index,
      })
      output[state.text_index + 1] = item_value
    end
    for _, tool in ipairs(state.tools) do
      if not function_name(tool.name) or tool.call_id == "" then
        return failure("upstream_contract_violation", "tool_calls", "stream_tool_incomplete")
      end
      if not tool.started then
        tool.output_index = state.output_count
        state.output_count = state.output_count + 1
        tool.started = true
        stream_event(events, "response.output_item.added", {
          output_index = tool.output_index,
          item = { id = tool.item_id, type = "function_call", status = "in_progress",
            call_id = tool.call_id, name = tool.name, arguments = "" },
        })
        if tool.arguments ~= "" then
          stream_event(events, "response.function_call_arguments.delta", {
            item_id = tool.item_id, output_index = tool.output_index,
            delta = tool.arguments,
          })
        end
      end
      local _, err = parse_arguments(tool.arguments, "tool_calls.arguments")
      if err then return failure("upstream_contract_violation", "tool_calls.arguments", "function_arguments_invalid") end
      stream_event(events, "response.function_call_arguments.done", {
        item_id = tool.item_id, output_index = tool.output_index,
        arguments = tool.arguments,
      })
      local item_value = {
        id = tool.item_id, type = "function_call", status = "completed",
        call_id = tool.call_id, name = tool.name, arguments = tool.arguments,
      }
      stream_event(events, "response.output_item.done", {
        item = item_value, output_index = tool.output_index,
      })
      output[tool.output_index + 1] = item_value
    end
    local incomplete = state.finish_reason == "length"
        or state.finish_reason == "content_filter"
    if state.finish_reason ~= "stop" and state.finish_reason ~= "tool_calls"
        and not incomplete then
      return failure("upstream_contract_violation", "finish_reason", "finish_reason_unknown")
    end
    local status = incomplete and "incomplete" or "completed"
    local reason = state.finish_reason == "content_filter"
        and "content_filter" or "max_output_tokens"
    stream_event(events, "response." .. status, {
      response = {
        id = state.id, object = "response", created_at = state.created,
        model = state.model, status = status,
        incomplete_details = incomplete and { reason = reason } or aster.null,
        output = output, usage = public_usage(usage, "responses"),
      },
    })
    return ok({ state = state, events = events, terminal = true })
  end
  local chunk = input.chunk
  if type(chunk) ~= "table" or not is_array(chunk.choices) or #chunk.choices > 1 then
    return failure("upstream_contract_violation", "choices", "stream_chunk_invalid")
  end
  for _, choice in ipairs(chunk.choices) do
    if choice.index ~= 0 or type(choice.delta) ~= "table" then
      return failure("upstream_contract_violation", "choices", "stream_choice_invalid")
    end
    local delta = choice.delta
    if is_present(delta.reasoning_content) then
      if type(delta.reasoning_content) ~= "string" then
        return failure("upstream_contract_violation", "delta.reasoning_content", "reasoning_text_invalid")
      end
      if not is_present(state.reasoning_index) then
        state.reasoning_index = state.output_count
        state.output_count = state.output_count + 1
        stream_event(events, "response.output_item.added", {
          output_index = state.reasoning_index,
          item = { id = state.id .. "-reasoning", type = "reasoning",
            status = "in_progress", summary = array({}) },
        })
      end
      state.reasoning = state.reasoning .. delta.reasoning_content
      stream_event(events, "response.reasoning_text.delta", {
        item_id = state.id .. "-reasoning", output_index = state.reasoning_index,
        content_index = 0, delta = delta.reasoning_content,
      })
    end
    if is_present(delta.content) then
      if type(delta.content) ~= "string" then
        return failure("upstream_contract_violation", "delta.content", "stream_text_invalid")
      end
      if not is_present(state.text_index) then
        state.text_index = state.output_count
        state.output_count = state.output_count + 1
        stream_event(events, "response.output_item.added", {
          output_index = state.text_index,
          item = { id = state.id .. "-message", type = "message",
            status = "in_progress", role = "assistant", content = array({}) },
        })
        stream_event(events, "response.content_part.added", {
          item_id = state.id .. "-message", output_index = state.text_index,
          content_index = 0, part = { type = "output_text", text = "", annotations = array({}) },
        })
      end
      state.text = state.text .. delta.content
      stream_event(events, "response.output_text.delta", {
        item_id = state.id .. "-message", output_index = state.text_index,
        content_index = 0, delta = delta.content,
      })
    end
    if is_present(delta.tool_calls) then
      if not is_array(delta.tool_calls) then
        return failure("upstream_contract_violation", "delta.tool_calls", "stream_tools_invalid")
      end
      for _, fragment in ipairs(delta.tool_calls) do
        if type(fragment.index) ~= "number" or fragment.index < 0
            or fragment.index > 127 or fragment.index % 1 ~= 0 then
          return failure("upstream_contract_violation", "tool_calls.index", "stream_tool_index_invalid")
        end
        local index = fragment.index + 1
        local tool = state.tools[index]
        if not tool then
          if index ~= #state.tools + 1 then
            return failure("upstream_contract_violation", "tool_calls.index", "stream_tool_order_invalid")
          end
          tool = { call_id = "", name = "", arguments = "", started = false,
            item_id = state.id .. "-call-" .. index }
          state.tools[index] = tool
        end
        if is_present(fragment.id) then
          if type(fragment.id) ~= "string" then
            return failure("upstream_contract_violation", "tool_calls.id", "stream_tool_id_invalid")
          end
          if tool.started and fragment.id ~= "" then
            return failure("upstream_contract_violation", "tool_calls.id", "stream_tool_id_changed")
          end
          tool.call_id = tool.call_id .. fragment.id
        end
        local function_fragment = fragment["function"]
        if is_present(function_fragment) then
          if type(function_fragment) ~= "table" then
            return failure("upstream_contract_violation", "tool_calls.function", "stream_tool_invalid")
          end
          if is_present(function_fragment.name) then
            if type(function_fragment.name) ~= "string" then
              return failure("upstream_contract_violation", "tool_calls.function.name", "stream_tool_name_invalid")
            end
            if tool.started and function_fragment.name ~= "" then
              return failure("upstream_contract_violation", "tool_calls.function.name", "stream_tool_name_changed")
            end
            tool.name = tool.name .. function_fragment.name
          end
          if is_present(function_fragment.arguments) then
            if type(function_fragment.arguments) ~= "string" then
              return failure("upstream_contract_violation", "tool_calls.function.arguments", "stream_tool_arguments_invalid")
            end
            tool.arguments = tool.arguments .. function_fragment.arguments
          end
        end
        if not tool.started and tool.call_id ~= "" and function_name(tool.name)
            and is_present(function_fragment)
            and type(function_fragment.arguments) == "string" then
          tool.output_index = state.output_count
          state.output_count = state.output_count + 1
          tool.started = true
          stream_event(events, "response.output_item.added", {
            output_index = tool.output_index,
            item = { id = tool.item_id, type = "function_call", status = "in_progress",
              call_id = tool.call_id, name = tool.name, arguments = "" },
          })
          if tool.arguments ~= "" then
            stream_event(events, "response.function_call_arguments.delta", {
              item_id = tool.item_id, output_index = tool.output_index,
              delta = tool.arguments,
            })
          end
        elseif tool.started and is_present(function_fragment)
            and type(function_fragment.arguments) == "string" then
          stream_event(events, "response.function_call_arguments.delta", {
            item_id = tool.item_id, output_index = tool.output_index,
            delta = function_fragment.arguments,
          })
        end
      end
    end
    if is_present(choice.finish_reason) then
      if is_present(state.finish_reason) then
        return failure("upstream_contract_violation", "finish_reason", "duplicate_finish_reason")
      end
      state.finish_reason = choice.finish_reason
    end
  end
  return ok({ state = state, events = events, terminal = false })
end

function M.map_chat_to_messages_stream(input)
  if type(input) ~= "table" or type(input.public_id) ~= "string"
      or type(input.public_model) ~= "string" then
    return failure("invalid_request", aster.null, "stream_mapping_invalid")
  end
  local state = input.state
  if not is_present(state) then
    state = { id = input.public_id, model = input.public_model,
      blocks = array({}), tools = array({}), finish_reason = aster.null }
  end
  if state.id ~= input.public_id or state.model ~= input.public_model then
    return failure("invalid_request", "state", "stream_state_mismatch")
  end
  local events = array({})
  if not state.started then
    stream_event(events, "message_start", { message = {
      id = state.id, type = "message", role = "assistant", model = state.model,
      content = array({}), stop_reason = aster.null, stop_sequence = aster.null,
      usage = { input_tokens = 0, output_tokens = 0 },
    } })
    state.started = true
  end
  if input.done == true then
    if not is_present(state.finish_reason) or not is_present(input.usage) then
      return failure("upstream_contract_violation", "finish_reason", "stream_terminal_incomplete")
    end
    local usage = usage_value(input.usage, "prompt_tokens", "cached_tokens",
      "completion_tokens", "reasoning_tokens")
    if type(usage) ~= "table" or not is_present(usage.input_tokens)
        or not is_present(usage.output_tokens) then
      return failure("upstream_contract_violation", "usage", "stream_usage_invalid")
    end
    if is_present(state.text_index) then
      stream_event(events, "content_block_stop", { index = state.text_index })
    end
    for _, tool in ipairs(state.tools) do
      if not function_name(tool.name) or tool.call_id == "" then
        return failure("upstream_contract_violation", "tool_calls", "stream_tool_incomplete")
      end
      local arguments, err = parse_arguments(tool.arguments, "tool_calls.arguments")
      if err then return failure("upstream_contract_violation", "tool_calls.arguments", "function_arguments_invalid") end
      tool.block_index = #state.blocks
      state.blocks[#state.blocks + 1] = { kind = "tool", tool_index = tool.index }
      stream_event(events, "content_block_start", {
        index = tool.block_index,
        content_block = { type = "tool_use", id = tool.call_id,
          name = tool.name, input = {} },
      })
      stream_event(events, "content_block_delta", {
        index = tool.block_index,
        delta = { type = "input_json_delta", partial_json = tool.arguments },
      })
      stream_event(events, "content_block_stop", { index = tool.block_index })
      tool.parsed_arguments = arguments
    end
    local reasons = { stop = "end_turn", tool_calls = "tool_use",
      length = "max_tokens", content_filter = "refusal" }
    local reason = reasons[state.finish_reason]
    if not reason then
      return failure("upstream_contract_violation", "finish_reason", "finish_reason_unknown")
    end
    stream_event(events, "message_delta", {
      delta = { stop_reason = reason, stop_sequence = aster.null },
      usage = public_usage(usage, "anthropic_messages"),
    })
    stream_event(events, "message_stop", {})
    return ok({ state = state, events = events, terminal = true })
  end
  local chunk = input.chunk
  if type(chunk) ~= "table" or not is_array(chunk.choices) or #chunk.choices > 1 then
    return failure("upstream_contract_violation", "choices", "stream_chunk_invalid")
  end
  for _, choice in ipairs(chunk.choices) do
    if choice.index ~= 0 or type(choice.delta) ~= "table" then
      return failure("upstream_contract_violation", "choices", "stream_choice_invalid")
    end
    local delta = choice.delta
    if is_present(delta.reasoning_content) and delta.reasoning_content ~= "" then
      return failure("upstream_contract_violation", "delta.reasoning_content", "reasoning_stream_not_portable")
    end
    if is_present(delta.content) then
      if type(delta.content) ~= "string" then
        return failure("upstream_contract_violation", "delta.content", "stream_text_invalid")
      end
      if not is_present(state.text_index) then
        state.text_index = #state.blocks
        state.blocks[#state.blocks + 1] = { kind = "text" }
        stream_event(events, "content_block_start", {
          index = state.text_index, content_block = { type = "text", text = "" },
        })
      end
      stream_event(events, "content_block_delta", {
        index = state.text_index,
        delta = { type = "text_delta", text = delta.content },
      })
    end
    if is_present(delta.tool_calls) then
      if not is_array(delta.tool_calls) then
        return failure("upstream_contract_violation", "delta.tool_calls", "stream_tools_invalid")
      end
      for _, fragment in ipairs(delta.tool_calls) do
        if type(fragment.index) ~= "number" or fragment.index < 0
            or fragment.index > 127 or fragment.index % 1 ~= 0 then
          return failure("upstream_contract_violation", "tool_calls.index", "stream_tool_index_invalid")
        end
        local index = fragment.index + 1
        local tool = state.tools[index]
        if not tool then
          if index ~= #state.tools + 1 then
            return failure("upstream_contract_violation", "tool_calls.index", "stream_tool_order_invalid")
          end
          tool = { index = index, call_id = "", name = "", arguments = "" }
          state.tools[index] = tool
        end
        if is_present(fragment.id) then
          if type(fragment.id) ~= "string" then
            return failure("upstream_contract_violation", "tool_calls.id", "stream_tool_id_invalid")
          end
          tool.call_id = tool.call_id .. fragment.id
        end
        local function_fragment = fragment["function"]
        if is_present(function_fragment) then
          if type(function_fragment) ~= "table" then
            return failure("upstream_contract_violation", "tool_calls.function", "stream_tool_invalid")
          end
          if is_present(function_fragment.name) then
            if type(function_fragment.name) ~= "string" then
              return failure("upstream_contract_violation", "tool_calls.function.name", "stream_tool_name_invalid")
            end
            tool.name = tool.name .. function_fragment.name
          end
          if is_present(function_fragment.arguments) then
            if type(function_fragment.arguments) ~= "string" then
              return failure("upstream_contract_violation", "tool_calls.function.arguments", "stream_tool_arguments_invalid")
            end
            tool.arguments = tool.arguments .. function_fragment.arguments
          end
        end
      end
    end
    if is_present(choice.finish_reason) then
      if is_present(state.finish_reason) then
        return failure("upstream_contract_violation", "finish_reason", "duplicate_finish_reason")
      end
      state.finish_reason = choice.finish_reason
    end
  end
  return ok({ state = state, events = events, terminal = false })
end

function M.map_responses_to_chat_stream(input)
  if type(input) ~= "table" or type(input.public_id) ~= "string"
      or type(input.public_model) ~= "string" then
    return failure("invalid_request", aster.null, "stream_mapping_invalid")
  end
  local state = input.state
  if not is_present(state) then
    state = { id = input.public_id, model = input.public_model,
      created = input.created or 0, tool_indices = {}, tool_count = 0,
      started = false }
  end
  if state.id ~= input.public_id or state.model ~= input.public_model then
    return failure("invalid_request", "state", "stream_state_mismatch")
  end
  local events = array({})
  local function chunk(delta, finish_reason, usage)
    events[#events + 1] = {
      type = "chat.chunk",
      chunk = { id = state.id, object = "chat.completion.chunk",
        created = state.created, model = state.model,
        choices = finish_reason and array({ { index = 0, delta = {},
          finish_reason = finish_reason } })
          or array({ { index = 0, delta = delta, finish_reason = aster.null } }),
        usage = usage or aster.null },
    }
  end
  local event = input.event
  if type(event) ~= "table" or type(event.type) ~= "string" then
    return failure("upstream_contract_violation", "event", "response_event_invalid")
  end
  local kind = event.type
  if kind == "response.failed" then
    return failure("upstream_failed", "response", "upstream_response_failed")
  elseif kind == "response.completed" or kind == "response.incomplete" then
    local response = event.response
    if type(response) ~= "table" then
      return failure("upstream_contract_violation", "response", "response_terminal_invalid")
    end
    local usage = usage_value(response.usage, "input_tokens", "cached_tokens",
      "output_tokens", "reasoning_tokens")
    if type(usage) ~= "table" or not is_present(usage.input_tokens)
        or not is_present(usage.output_tokens) then
      return failure("upstream_contract_violation", "usage", "stream_usage_invalid")
    end
    local reason = "stop"
    if kind == "response.incomplete" then
      reason = type(response.incomplete_details) == "table"
        and response.incomplete_details.reason == "content_filter"
        and "content_filter" or "length"
    elseif is_array(response.output) then
      for _, item_value in ipairs(response.output) do
        if item_value.type == "function_call" then reason = "tool_calls" end
      end
    end
    chunk({}, reason, public_usage(usage, "chat_completions"))
    return ok({ state = state, events = events, terminal = true })
  elseif kind == "response.output_text.delta" then
    if type(event.delta) ~= "string" then
      return failure("upstream_contract_violation", "delta", "stream_text_invalid")
    end
    chunk({ content = event.delta }, nil, nil)
  elseif kind == "response.reasoning_text.delta" then
    if type(event.delta) ~= "string" then
      return failure("upstream_contract_violation", "delta", "reasoning_text_invalid")
    end
    chunk({ reasoning_content = event.delta }, nil, nil)
  elseif kind == "response.output_item.added" then
    local item_value = event.item
    if type(item_value) == "table" and item_value.type == "function_call" then
      if type(item_value.id) ~= "string" or type(item_value.call_id) ~= "string"
          or not function_name(item_value.name) then
        return failure("upstream_contract_violation", "item", "stream_tool_invalid")
      end
      if is_present(state.tool_indices[item_value.id]) then
        return failure("upstream_contract_violation", "item.id", "stream_tool_duplicate")
      end
      local index = state.tool_count
      state.tool_indices[item_value.id] = index
      state.tool_count = index + 1
      chunk({ tool_calls = array({ { index = index, id = item_value.call_id,
        type = "function", ["function"] = { name = item_value.name, arguments = "" } } }) }, nil, nil)
    end
  elseif kind == "response.function_call_arguments.delta" then
    local index = state.tool_indices[event.item_id]
    if not is_present(index) or type(event.delta) ~= "string" then
      return failure("upstream_contract_violation", "item_id", "stream_tool_delta_invalid")
    end
    chunk({ tool_calls = array({ { index = index,
      ["function"] = { arguments = event.delta } } }) }, nil, nil)
  elseif kind == "response.created" or kind == "response.in_progress"
      or kind == "response.output_item.done"
      or kind == "response.content_part.added" or kind == "response.content_part.done"
      or kind == "response.output_text.done"
      or kind == "response.function_call_arguments.done"
      or kind == "response.reasoning_text.done"
      or kind == "response.reasoning_summary_part.added"
      or kind == "response.reasoning_summary_part.done"
      or kind == "response.reasoning_summary_text.delta"
      or kind == "response.reasoning_summary_text.done" then
    -- These events do not carry a new public Chat delta.
  else
    return failure("upstream_contract_violation", "event.type", "stream_event_unsupported")
  end
  return ok({ state = state, events = events, terminal = false })
end

public_usage = function(usage, protocol)
  if not is_present(usage) then return nil end
  local input = usage.input_tokens
  local output = usage.output_tokens
  if not is_present(input) or not is_present(output) then return nil end
  if protocol == "chat_completions" then
    local details = nil
    if is_present(usage.cached_input_tokens) then
      details = { cached_tokens = usage.cached_input_tokens }
    end
    return {
      prompt_tokens = input, completion_tokens = output,
      total_tokens = input + output,
      prompt_tokens_details = details,
    }
  elseif protocol == "responses" then
    local details = nil
    if is_present(usage.cached_input_tokens) then
      details = { cached_tokens = usage.cached_input_tokens }
    end
    return {
      input_tokens = input, output_tokens = output,
      total_tokens = input + output,
      input_tokens_details = details,
    }
  else
    return { input_tokens = input, output_tokens = output }
  end
end

function M.encode_public(input)
  if type(input) ~= "table" or type(input.result) ~= "table"
      or type(input.protocol) ~= "string" or type(input.public_id) ~= "string" then
    return failure("invalid_request", aster.null, "public_encoding_invalid")
  end
  local result = input.result
  if result.schema_version ~= 2 or not is_array(result.output)
      or type(result.public_model) ~= "string" then
    return failure("invalid_request", "result", "canonical_result_invalid")
  end
  local text_parts = array({})
  local reasoning_parts = array({})
  local calls = array({})
  for _, output in ipairs(result.output) do
    if output.kind == "text" then
      text_parts[#text_parts + 1] = output.text
    elseif output.kind == "tool_call" then
      calls[#calls + 1] = output
    elseif output.kind == "reasoning_text" then
      reasoning_parts[#reasoning_parts + 1] = output.text
    else
      return failure("unsupported_operation", "result.output", "output_kind_not_supported")
    end
  end
  local answer = table.concat(text_parts)
  local usage = public_usage(result.usage, input.protocol)
  if input.protocol == "chat_completions" then
    local message = { role = "assistant", content = answer ~= "" and answer or aster.null }
    if #calls > 0 then
      local tool_calls = array({})
      for _, call in ipairs(calls) do
        tool_calls[#tool_calls + 1] = {
          id = call.call_id, type = "function",
          ["function"] = { name = call.name, arguments = aster.stringify_json(call.arguments) },
        }
      end
      message.tool_calls = tool_calls
    end
    if #reasoning_parts > 0 then
      message.reasoning_content = table.concat(reasoning_parts)
    end
    return ok({
      id = input.public_id, object = "chat.completion", created = input.created or 0,
      model = result.public_model,
      choices = array({ { index = 0, message = message, finish_reason = result.finish_reason } }),
      usage = usage,
    })
  elseif input.protocol == "responses" then
    local output = array({})
    if #reasoning_parts > 0 then
      output[#output + 1] = {
        id = input.public_id .. "-reasoning", type = "reasoning", status = "completed",
        summary = array({ { type = "summary_text", text = table.concat(reasoning_parts) } }),
      }
    end
    if answer ~= "" then
      output[#output + 1] = {
        id = input.public_id .. "-message", type = "message", status = "completed",
        role = "assistant", content = array({ { type = "output_text", text = answer,
          annotations = array({}) } }),
      }
    end
    for index, call in ipairs(calls) do
      output[#output + 1] = {
        id = input.public_id .. "-call-" .. index, type = "function_call", status = "completed",
        call_id = call.call_id, name = call.name,
        arguments = aster.stringify_json(call.arguments),
      }
    end
    local incomplete = result.finish_reason == "length" or result.finish_reason == "content_filter"
    local incomplete_reason = result.finish_reason == "content_filter" and "content_filter" or "max_output_tokens"
    return ok({
      id = input.public_id, object = "response", created_at = input.created or 0,
      model = result.public_model,
      status = incomplete and "incomplete" or "completed",
      incomplete_details = incomplete and { reason = incomplete_reason } or aster.null,
      output = output, usage = usage,
    })
  elseif input.protocol == "anthropic_messages" then
    if #reasoning_parts > 0 then
      return failure("unsupported_operation", "result.output", "reasoning_history_not_portable")
    end
    local content = array({})
    if answer ~= "" then content[#content + 1] = { type = "text", text = answer } end
    for _, call in ipairs(calls) do
      content[#content + 1] = {
        type = "tool_use", id = call.call_id, name = call.name, input = call.arguments,
      }
    end
    local reasons = { stop = "end_turn", tool_calls = "tool_use", length = "max_tokens",
      content_filter = "refusal" }
    local stop_reason = reasons[result.finish_reason]
    if not stop_reason then
      return failure("unsupported_operation", "result.finish_reason", "finish_reason_not_supported")
    end
    return ok({
      id = input.public_id, type = "message", role = "assistant",
      model = result.public_model, content = content,
      stop_reason = stop_reason, stop_sequence = aster.null,
      usage = usage,
    })
  end
  return failure("unsupported_operation", "protocol", "public_protocol_not_supported")
end

-- Images use the same signed adapter, but their quota and output are independent
-- of token usage. One invocation always requests one image; Control owns n.
function M.prepare_image(input)
  if type(input) ~= "table" or type(input.body) ~= "table"
      or type(input.target) ~= "table" then
    return failure("invalid_request", aster.null, "image_request_invalid")
  end
  local channel = channels[input.target.channel_id]
  if not channel or channel.provider == "deepseek" then
    return failure("unsupported_channel", "model", "image_channel_not_supported")
  end
  local edit = input.edit == true
  local body = input.body
  local unknown = reject_unknown(body, {
    model=true, prompt=true, n=true, size=true, quality=true,
    background=true, moderation=true, output_format=true,
    output_compression=true, input_fidelity=true, response_format=true,
  })
  if unknown then return unknown end
  if type(body.prompt) ~= "string" or #body.prompt == 0 or #body.prompt > 32000
      or type(body.model) ~= "string" or body.model == ""
      or (is_present(body.n) and (type(body.n) ~= "number" or body.n < 1
          or body.n > 10 or body.n % 1 ~= 0))
      or (is_present(body.response_format) and body.response_format ~= "b64_json") then
    return failure("invalid_request", "prompt", "image_fields_invalid")
  end
  local model = input.target.upstream_model
  if type(model) ~= "string" or model == "" then
    return failure("invalid_request", "model", "upstream_model_missing")
  end
  local rule = public_rules.image_models and public_rules.image_models[model]
  if not rule or rule.provider ~= channel.provider or (edit and not rule.edit)
      or (not edit and not rule.generate) then
    return failure("unsupported_model", "model", "image_operation_not_supported")
  end
  for _, key in ipairs({"size", "quality", "background", "moderation",
      "output_format", "input_fidelity"}) do
    if is_present(body[key]) and type(body[key]) ~= "string" then
      return failure("invalid_request", key, "image_field_type_invalid")
    end
  end
  if is_present(body.output_compression)
      and (type(body.output_compression) ~= "number"
        or body.output_compression % 1 ~= 0
        or body.output_compression < 0 or body.output_compression > 100) then
    return failure("invalid_request", "output_compression", "image_compression_invalid")
  end
  if is_present(body.quality) and body.quality ~= "auto"
      and body.quality ~= "low" and body.quality ~= "medium"
      and body.quality ~= "high" and body.quality ~= "xhigh"
      and body.quality ~= "max" then
    return failure("unsupported_parameter", "quality", "image_quality_not_supported")
  end
  if is_present(body.size) and body.size ~= "auto" then
    local width, height = body.size:match("^(%d+)x(%d+)$")
    width, height = tonumber(width), tonumber(height)
    if not width or not height or width < 1 or height < 1
        or width > 3840 or height > 3840 then
      return failure("invalid_request", "size", "image_size_invalid")
    end
    if channel.provider == "glm" then
      local minimum, divisor, maximum = 512, 16, 2097152
      if model == "glm-image" then minimum, divisor, maximum = 1024, 32, 4194304 end
      if width < minimum or height < minimum or width > 2048 or height > 2048
          or width % divisor ~= 0 or height % divisor ~= 0
          or width * height > maximum then
        return failure("unsupported_parameter", "size", "glm_image_size_not_supported")
      end
    elseif model == "gpt-image-1" or model == "gpt-image-1.5"
        or model == "gpt-image-1-mini" or model == "chatgpt-image-latest" then
      if body.size ~= "1024x1024" and body.size ~= "1536x1024"
          and body.size ~= "1024x1536" then
        return failure("unsupported_parameter", "size", "openai_image_size_not_supported")
      end
    elseif width % 16 ~= 0 or height % 16 ~= 0
        or math.max(width, height) > 3840 or math.min(width, height) > 2160
        or width * height > 8294400 or width > height * 3 or height > width * 3 then
      return failure("unsupported_parameter", "size", "openai_image_size_not_supported")
    end
  end
  local upstream = { model = model, prompt = body.prompt }
  local path
  if channel.provider == "openai" then
    if (body.quality == "xhigh" or body.quality == "max")
        and model:sub(1, 14) ~= "gpt-image-2.5-" then
      return failure("unsupported_parameter", "quality", "openai_image_quality_not_supported")
    end
    if is_present(body.background) and body.background ~= "auto"
        and body.background ~= "opaque" and body.background ~= "transparent" then
      return failure("unsupported_parameter", "background", "openai_image_background_not_supported")
    end
    if body.background == "transparent" and body.output_format == "jpeg" then
      return failure("unsupported_parameter", "output_format", "transparent_jpeg_not_supported")
    end
    if is_present(body.moderation) and body.moderation ~= "auto"
        and body.moderation ~= "low" then
      return failure("unsupported_parameter", "moderation", "openai_image_moderation_not_supported")
    end
    if is_present(body.output_format) and body.output_format ~= "png"
        and body.output_format ~= "jpeg" and body.output_format ~= "webp" then
      return failure("unsupported_parameter", "output_format", "openai_image_format_not_supported")
    end
    if is_present(body.output_compression)
        and body.output_format ~= "jpeg" and body.output_format ~= "webp" then
      return failure("unsupported_parameter", "output_compression", "compression_requires_jpeg_or_webp")
    end
    if is_present(body.input_fidelity) and body.input_fidelity ~= "high"
        and body.input_fidelity ~= "low" then
      return failure("unsupported_parameter", "input_fidelity", "openai_input_fidelity_not_supported")
    end
    for _, key in ipairs({"size", "quality", "background", "moderation",
        "output_format", "output_compression", "input_fidelity"}) do
      local value = body[key]
      if is_present(value) and value ~= "auto" and value ~= "" then
        upstream[key] = value
      end
    end
    if not edit and is_present(body.input_fidelity) then
      return failure("unsupported_parameter", "input_fidelity", "edit_only")
    end
    path = edit and "/images/edits" or "/images/generations"
    if edit then
      if not is_array(input.sources) or #input.sources == 0 or #input.sources > 16 then
        return failure("invalid_request", "image", "image_source_required")
      end
      upstream.__image_sources = input.sources
      if is_present(input.mask) then upstream.__image_mask = input.mask end
    end
  else
    if edit then
      return failure("unsupported_operation", "image", "glm_image_edit_not_supported")
    end
    if model ~= "glm-image" and model ~= "cogview-4-250304" then
      return failure("unsupported_model", "model", "glm_image_model_required")
    end
    for _, key in ipairs({"background", "moderation", "output_format",
        "output_compression", "input_fidelity"}) do
      if is_present(body[key]) and body[key] ~= "auto" and body[key] ~= "" then
        return failure("unsupported_parameter", key, "glm_image_field_not_supported")
      end
    end
    if is_present(body.size) and body.size ~= "auto" then upstream.size = body.size end
    if is_present(body.quality) and body.quality ~= "auto" then
      local quality = ({high="hd", medium="standard"})[body.quality]
      if not quality then
        return failure("unsupported_parameter", "quality", "glm_quality_not_supported")
      end
      upstream.quality = quality
    end
    path = "/images/generations"
  end
  return ok({
    action="execute", method="POST", endpoint_id=channel.profile,
    relative_path=path,
    public_headers=array({{"content-type", edit and "multipart/form-data" or "application/json"}}),
    secret_bindings=array({{slot="api_key", destination="authorization_bearer"}}),
    body=upstream, response_mode="json", timeout_ms=120000,
    redirect_policy="deny",
  })
end

function M.parse_image(input)
  if type(input) ~= "table" or type(input.body) ~= "table" then
    return failure("upstream_contract_violation", "body", "image_response_invalid")
  end
  local data = input.body.data
  if not is_array(data) or #data ~= 1 or type(data[1]) ~= "table" then
    return failure("upstream_contract_violation", "data", "one_image_required")
  end
  local item = data[1]
  if input.provider == "glm" and type(item.url) == "string" and #item.url > 0 then
    return ok({ kind="url", value=item.url,
      revised_prompt=item.revised_prompt or aster.null })
  end
  if input.provider == "openai" and type(item.b64_json) == "string"
      and #item.b64_json > 0 then
    return ok({ kind="base64", value=item.b64_json,
      revised_prompt=item.revised_prompt or aster.null })
  end
  return failure("upstream_contract_violation", "data[1]", "image_content_missing")
end

-- Codex uses OAuth material owned by the host. Only its protocol body crosses
-- this boundary; access tokens and account identifiers never enter Lua.
function M.prepare_codex(input)
  if type(input) ~= "table" or type(input.body) ~= "table"
      or type(input.upstream_model) ~= "string" or input.upstream_model == "" then
    return failure("invalid_request", "body", "codex_request_invalid")
  end
  local body = input.body
  if is_present(body.max_output_tokens) then
    return failure("unsupported_parameter", "max_output_tokens", "codex_output_limit_unavailable")
  end
  if type(body.input) == "string" then
    body.input = array({{role="user",content=array({{type="input_text",text=body.input}})}})
  elseif type(body.input) == "table" and not is_array(body.input) then
    body.input = array({body.input})
  elseif not is_array(body.input) then
    return failure("invalid_request", "input", "codex_input_invalid")
  end
  if body.service_tier == "fast" then body.service_tier = "priority" end
  body.model = input.upstream_model
  body.stream = true
  body.store = false
  return ok(body)
end

function M.self_test()
  local decoded = M.decode_request({
    protocol = "responses",
    body = { model = "fixture", input = "hello", store = false },
  })
  if decoded.status ~= "ok" then return false end
  if provider == "deepseek" then
    local target = {
      channel_id = "deepseek.api",
      connection_id = "fixture",
      connection_revision = 1,
      upstream_model = "deepseek-flash",
    }
    local plan = M.assess({
      operation = decoded.value, target = target, compatibility_mode = "compatible",
    })
    if plan.status ~= "ok" or not plan.value.compatible then return false end
    local prepared = M.prepare({
      operation = decoded.value, target = target, plan = plan.value,
    })
    if prepared.status ~= "ok" or prepared.value.endpoint_id ~= "deepseek.api" then return false end
  end
  local rejected = M.decode_request({
    protocol = "responses", body = { model = "fixture", input = "hello", previous_response_id = "x" },
  })
  if rejected.status ~= "error" or rejected.error.code ~= "unsupported_parameter" then return false end
  local tools = M.decode_request({
    protocol = "chat_completions",
    body = {
      model = "fixture",
      messages = array({
        { role = "assistant", tool_calls = array({ { id = "call_1", type = "function",
          ["function"] = { name = "lookup", arguments = "{}" } } }) },
        { role = "tool", tool_call_id = "call_1", content = "found" },
      }),
      tools = array({ { type = "function", ["function"] = {
        name = "lookup", parameters = { type = "object" },
      } } }),
    },
  })
  if tools.status ~= "ok" then return false end
  if provider == "glm" then
    local tool_plan = M.assess({
      operation = tools.value,
      target = { channel_id = "glm.zai.general", connection_id = "fixture",
        connection_revision = 1, upstream_model = "glm-fixture" },
      compatibility_mode = "compatible",
    })
    if tool_plan.status ~= "ok" or not tool_plan.value.compatible then return false end
  end
  local upstream = M.parse_buffered({
    wire_protocol = "chat", public_model = "fixture",
    body = { id = "upstream-id", choices = array({ {
      finish_reason = "stop", message = { role = "assistant", content = "answer" },
    } }) },
  })
  if upstream.status ~= "ok" then return false end
  local public = M.encode_public({
    protocol = "responses", public_id = "resp-fixture", result = upstream.value,
  })
  return public.status == "ok" and public.value.output[1].content[1].text == "answer"
end

return M
