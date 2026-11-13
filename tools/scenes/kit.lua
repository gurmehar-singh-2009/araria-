-- Helpers shared by the scene builders in this folder.
--
-- `kit.make(class, props, parent)` creates an instance from a table:
--   {1, 2, 3}        becomes a Vector3      "#ff8800"  becomes a Color3
--   {0.5, 0.5}       becomes a Vector2      {0, 10, 0, 20} becomes a UDim2
-- plus the special keys Tags = {...}, Attributes = {...} and
-- Children = { {class, props}, ... }.

local kit = {}

local function convert(value)
    if type(value) == "table" then
        if #value == 3 then return Vector3.new(value[1], value[2], value[3]) end
        if #value == 2 then return Vector2.new(value[1], value[2]) end
        if #value == 4 then return UDim2.new(value[1], value[2], value[3], value[4]) end
    elseif type(value) == "string" and value:sub(1, 1) == "#" and (#value == 7 or #value == 4) then
        return Color3.fromHex(value)
    end
    return value
end

function kit.make(class, props, parent)
    local instance = Instance.new(class)
    for key, value in pairs(props or {}) do
        if key == "Tags" then
            for _, tag in ipairs(value) do instance:AddTag(tag) end
        elseif key == "Attributes" then
            for name, attribute in pairs(value) do instance:SetAttribute(name, convert(attribute)) end
        elseif key == "Children" then
            -- created below, once the instance is configured
        else
            instance[key] = convert(value)
        end
    end
    for _, child in ipairs(props and props.Children or {}) do
        kit.make(child[1], child[2], instance)
    end
    instance.Parent = parent or workspace
    return instance
end

function kit.folder(name, parent)
    return kit.make("Folder", { Name = name }, parent)
end

-- A primitive part. `shape` defaults to a block.
function kit.part(name, position, size, color, extra, parent)
    local props = { Name = name, Position = position, Size = size, Color = color }
    for key, value in pairs(extra or {}) do props[key] = value end
    return kit.make("Part", props, parent)
end

-- A model from the asset library, standing on `position`.
function kit.mesh(asset, position, extra, parent)
    local props = { Asset = "models/" .. asset, Position = position, Name = asset }
    for key, value in pairs(extra or {}) do props[key] = value end
    return kit.make("MeshPart", props, parent)
end

-- Registers a game script stored in the game's scripts/ folder.
function kit.script(name)
    return kit.make("Script", { Name = name, Source = "scripts/" .. name .. ".lua" }, Scripts)
end

function kit.module(name)
    return kit.make("ModuleScript", { Name = name, Source = "scripts/" .. name .. ".lua" }, Scripts)
end

function kit.spawn(position, facing, parent)
    return kit.make("SpawnPoint", { Position = position, Size = { 3.4, 0.25, 3.4 }, Color = "#4cc9f0", Rotation = { 0, facing or 0, 0 } }, parent)
end

function kit.lighting(props)
    for key, value in pairs(props) do Lighting[key] = convert(value) end
end

-- A deterministic random source, so baked scenes are reproducible.
function kit.random(seed)
    local state = seed
    return function(low, high)
        state = (state * 1103515245 + 12345) % 2147483648
        local unit = state / 2147483648
        if not low then return unit end
        return low + (high - low) * unit
    end
end

return kit
