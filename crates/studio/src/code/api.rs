//! The scripting API reference.
//!
//! One description of everything scripts can use — instance classes come
//! from the engine's class schema, global functions and libraries from the
//! tables below. Autocomplete, signature hints, the in-editor reference
//! panel and `docs/lua-api.md` are all generated from it.

use araria_core::schema::{ClassInfo, PropType, schema};

/// A documented function, value or field.
#[derive(Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    /// Parameters and result, e.g. `(seconds: number?): number`. Empty for values.
    pub signature: &'static str,
    pub doc: &'static str,
}

const fn e(name: &'static str, signature: &'static str, doc: &'static str) -> Entry {
    Entry { name, signature, doc }
}

/// A global table of functions and values, such as `task` or `Vector3`.
pub struct Library {
    pub name: &'static str,
    pub doc: &'static str,
    pub members: &'static [Entry],
    /// Members reached through a value of this type (`v.Magnitude`,
    /// `v:Dot(other)`) rather than through the library table.
    pub instance_members: &'static [Entry],
}

pub const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Global functions and values.
pub const GLOBALS: &[Entry] = &[
    e("game", "", "The root of the game tree. `game:GetService(name)` returns a service."),
    e("workspace", "", "The Workspace service: everything in the 3D world."),
    e("Workspace", "", "The Workspace service: everything in the 3D world."),
    e("Players", "", "The Players service. `Players.LocalPlayer` is the player on this device."),
    e("Lighting", "", "Sun, sky, ambient light and fog."),
    e("Storage", "", "Templates to clone into the world; contents are not rendered or simulated."),
    e("Gui", "", "On-screen interface. Parent Frames, TextLabels and TextButtons here."),
    e("Scripts", "", "Container for scripts that are not attached to an object."),
    e("Camera", "", "The player's view."),
    e("script", "", "The Script instance running this code."),
    e("state", "", "A table that survives hot reloads of this script. Keep scores and progress here."),
    e("shared", "", "A table shared by every script in the game."),
    e("print", "(...: any)", "Writes values to the output."),
    e("warn", "(...: any)", "Writes a warning to the output."),
    e("error", "(message: any, level: number?)", "Stops the current script thread with an error."),
    e("assert", "(condition: any, message: string?): any", "Raises an error if `condition` is false or nil."),
    e("typeof", "(value: any): string", "The type name, including engine types such as `Vector3` and `Instance`."),
    e("type", "(value: any): string", "The basic Lua type name."),
    e("tostring", "(value: any): string", "Converts a value to text."),
    e("tonumber", "(value: any, base: number?): number?", "Converts text to a number, or returns nil."),
    e("pairs", "(t: table)", "Iterates over every key and value of a table."),
    e("ipairs", "(t: table)", "Iterates over the array part of a table in order."),
    e("next", "(t: table, key: any?): (any, any)", "Returns the entry after `key`."),
    e("select", "(n: number | \"#\", ...: any): ...any", "Returns the arguments from position `n`, or their count for `\"#\"`."),
    e("pcall", "(f: function, ...: any): (boolean, ...any)", "Calls a function, catching errors."),
    e("xpcall", "(f: function, handler: function, ...: any): (boolean, ...any)", "Calls a function with an error handler."),
    e("unpack", "(t: table, i: number?, j: number?): ...any", "Returns the elements of an array."),
    e("rawget", "(t: table, key: any): any", "Reads a table field without metamethods."),
    e("rawset", "(t: table, key: any, value: any): table", "Writes a table field without metamethods."),
    e("rawequal", "(a: any, b: any): boolean", "Compares without metamethods."),
    e("rawlen", "(t: table | string): number", "Length without metamethods."),
    e("setmetatable", "(t: table, metatable: table?): table", "Sets a table's metatable."),
    e("getmetatable", "(t: any): table?", "Returns a value's metatable."),
    e("require", "(module: string | ModuleScript): any", "Loads a module once and returns what it returned. `require(\"util\")` loads `scripts/util.lua`."),
    e("wait", "(seconds: number?): number", "Pauses this thread; returns the time actually waited. Same as `task.wait`."),
    e("spawn", "(f: function, ...: any): thread", "Runs a function in a new thread right away. Same as `task.spawn`."),
    e("delay", "(seconds: number, f: function, ...: any): thread", "Runs a function after a delay. Same as `task.delay`."),
    e("time", "(): number", "Seconds since the game started."),
    e("tick", "(): number", "Seconds since the game started."),
];

pub const LIBRARIES: &[Library] = &[
    Library {
        name: "Instance",
        doc: "Creates objects.",
        members: &[e("new", "(className: string, parent: Instance?): Instance", "Creates an instance of a class, optionally parented straight away.")],
        instance_members: &[],
    },
    Library {
        name: "task",
        doc: "The scheduler: waiting and running code concurrently.",
        members: &[
            e("wait", "(seconds: number?): number", "Pauses this thread for at least `seconds` (one frame by default)."),
            e("spawn", "(f: function, ...: any): thread", "Runs a function in a new thread immediately."),
            e("defer", "(f: function, ...: any): thread", "Runs a function in a new thread later this frame."),
            e("delay", "(seconds: number, f: function, ...: any): thread", "Runs a function in a new thread after a delay."),
            e("cancel", "(thread: thread)", "Stops a thread created by spawn, defer or delay."),
        ],
        instance_members: &[],
    },
    Library {
        name: "Vector3",
        doc: "A 3D vector. Supports `+ - * /` with vectors and numbers, unary minus and `==`.",
        members: &[
            e("new", "(x: number?, y: number?, z: number?): Vector3", "Creates a vector."),
            e("zero", "", "Vector3.new(0, 0, 0)"),
            e("one", "", "Vector3.new(1, 1, 1)"),
            e("xAxis", "", "Vector3.new(1, 0, 0)"),
            e("yAxis", "", "Vector3.new(0, 1, 0)"),
            e("zAxis", "", "Vector3.new(0, 0, 1)"),
        ],
        instance_members: &[
            e("X", "", "The x component."),
            e("Y", "", "The y component."),
            e("Z", "", "The z component."),
            e("Magnitude", "", "The vector's length."),
            e("Unit", "", "The vector scaled to length 1."),
            e("Dot", "(other: Vector3): number", "Dot product."),
            e("Cross", "(other: Vector3): Vector3", "Cross product."),
            e("Lerp", "(goal: Vector3, alpha: number): Vector3", "Blends toward `goal`."),
            e("FuzzyEq", "(other: Vector3, epsilon: number?): boolean", "Whether two vectors are nearly equal."),
        ],
    },
    Library {
        name: "Vector2",
        doc: "A 2D vector, used for screen positions and sizes.",
        members: &[e("new", "(x: number?, y: number?): Vector2", "Creates a vector."), e("zero", "", "Vector2.new(0, 0)"), e("one", "", "Vector2.new(1, 1)")],
        instance_members: &[e("X", "", "The x component."), e("Y", "", "The y component."), e("Magnitude", "", "The vector's length."), e("Unit", "", "The vector scaled to length 1.")],
    },
    Library {
        name: "Color3",
        doc: "A color with red, green and blue components from 0 to 1.",
        members: &[
            e("new", "(r: number, g: number, b: number): Color3", "From components between 0 and 1."),
            e("fromRGB", "(r: number, g: number, b: number): Color3", "From components between 0 and 255."),
            e("fromHSV", "(h: number, s: number, v: number): Color3", "From hue, saturation and value between 0 and 1."),
            e("fromHex", "(hex: string): Color3", "From a hex string such as `\"#ff8800\"`."),
        ],
        instance_members: &[
            e("R", "", "Red, 0 to 1."),
            e("G", "", "Green, 0 to 1."),
            e("B", "", "Blue, 0 to 1."),
            e("Lerp", "(goal: Color3, alpha: number): Color3", "Blends toward `goal`."),
            e("ToHex", "(): string", "The color as `#rrggbb`."),
            e("ToHSV", "(): (number, number, number)", "Hue, saturation and value."),
        ],
    },
    Library {
        name: "UDim2",
        doc: "An interface position or size: a fraction of the parent plus a pixel offset on each axis.",
        members: &[
            e("new", "(xScale: number, xOffset: number, yScale: number, yOffset: number): UDim2", "Creates a UDim2."),
            e("fromScale", "(x: number, y: number): UDim2", "Fractions of the parent only."),
            e("fromOffset", "(x: number, y: number): UDim2", "Pixels only."),
        ],
        instance_members: &[
            e("XScale", "", "Horizontal fraction of the parent."),
            e("XOffset", "", "Horizontal pixels."),
            e("YScale", "", "Vertical fraction of the parent."),
            e("YOffset", "", "Vertical pixels."),
            e("Lerp", "(goal: UDim2, alpha: number): UDim2", "Blends toward `goal`."),
        ],
    },
    Library {
        name: "CFrame",
        doc: "A position and orientation. `cf * vector` transforms a point, `cf * cf` combines frames, `cf + vector` offsets.",
        members: &[
            e("new", "(position: Vector3 | number?, lookAt: Vector3 | number?, z: number?): CFrame", "From a position, a position looking at a point, or x, y, z."),
            e("lookAt", "(position: Vector3, target: Vector3): CFrame", "At `position`, facing `target`."),
            e("Angles", "(x: number, y: number, z: number): CFrame", "A rotation from angles in radians."),
            e("fromOrientation", "(x: number, y: number, z: number): CFrame", "A rotation from angles in degrees."),
        ],
        instance_members: &[
            e("Position", "", "The position."),
            e("X", "", "The position's x."),
            e("Y", "", "The position's y."),
            e("Z", "", "The position's z."),
            e("Rotation", "", "Orientation as X/Y/Z angles in degrees."),
            e("LookVector", "", "The direction the frame faces."),
            e("RightVector", "", "The frame's right direction."),
            e("UpVector", "", "The frame's up direction."),
            e("Inverse", "(): CFrame", "The frame that undoes this one."),
            e("Lerp", "(goal: CFrame, alpha: number): CFrame", "Blends position and rotation toward `goal`."),
            e("PointToWorldSpace", "(point: Vector3): Vector3", "Transforms a local point to world space."),
            e("PointToObjectSpace", "(point: Vector3): Vector3", "Transforms a world point to local space."),
        ],
    },
    Library {
        name: "RunService",
        doc: "Per-frame events.",
        members: &[
            e("Heartbeat", "(dt: number)", "Event fired every frame with the seconds since the last one."),
            e("Stepped", "(dt: number)", "Alias of Heartbeat."),
            e("RenderStepped", "(dt: number)", "Alias of Heartbeat."),
            e("IsRunning", "(): boolean", "Whether the game is playing."),
        ],
        instance_members: &[],
    },
    Library {
        name: "Input",
        doc: "Keyboard and mouse. Key names: letters `A`–`Z`, `Zero`–`Nine`, `Space`, `LeftShift`, `Enter`, `Up`, `Down`, `Left`, `Right`, ...",
        members: &[
            e("KeyDown", "(key: string)", "Event fired when a key is pressed."),
            e("KeyUp", "(key: string)", "Event fired when a key is released."),
            e("MouseButtonDown", "(button: number, position: Vector2)", "Event fired when a mouse button is pressed (1 left, 2 right, 3 middle)."),
            e("MouseButtonUp", "(button: number, position: Vector2)", "Event fired when a mouse button is released."),
            e("MouseWheel", "(delta: number)", "Event fired when the wheel turns."),
            e("IsKeyDown", ":(key: string): boolean", "Whether a key is held."),
            e("IsMouseButtonDown", ":(button: number): boolean", "Whether a mouse button is held."),
            e("GetMousePosition", ":(): Vector2", "Pointer position in viewport pixels."),
            e("GetMouseDelta", ":(): Vector2", "Pointer movement this frame."),
            e("GetViewportSize", ":(): Vector2", "Size of the game view in pixels."),
            e("GetMoveVector", ":(): Vector3", "The movement keys as a world direction relative to the camera."),
            e("GetMoveAxis", ":(): Vector2", "The movement keys as (right, forward), each -1 to 1."),
            e("GetMouseRay", ":(): (Vector3, Vector3)", "Origin and direction of the ray through the pointer."),
            e("GetMouseHit", ":(ignore: {Instance}?): RaycastResult?", "What the pointer is over in the world."),
        ],
        instance_members: &[],
    },
    Library {
        name: "TweenService",
        doc: "Animates properties smoothly.",
        members: &[e("Create", ":(instance: Instance, info: TweenInfo | number, goals: {[string]: any}): Tween", "Creates a tween toward the given property values. Call `:Play()` on the result; it also has `:Pause()`, `:Cancel()` and a `Completed` event.")],
        instance_members: &[e("Play", "()", "Starts or resumes the tween."), e("Pause", "()", "Pauses the tween."), e("Cancel", "()", "Stops and discards the tween."), e("Completed", "()", "Event fired when the tween finishes.")],
    },
    Library {
        name: "TweenInfo",
        doc: "Timing options for a tween.",
        members: &[e("new", "(time: number?, style: string?, direction: string?, repeatCount: number?, reverses: boolean?, delayTime: number?): TweenInfo", "Styles: Linear, Quad, Cubic, Sine, Back, Bounce, Elastic, ... Directions: In, Out, InOut. A repeat count of -1 repeats forever.")],
        instance_members: &[],
    },
    Library {
        name: "Audio",
        doc: "Plays sounds from the asset library.",
        members: &[
            e("Play", ":(asset: string, volume: number?, pitch: number?)", "Plays a sound effect, e.g. `Audio:Play(\"audio/coin\")`."),
            e("PlayAt", ":(asset: string, position: Vector3, volume: number?)", "Plays a sound that is quieter far from the camera."),
            e("PlayMusic", ":(asset: string, volume: number?)", "Starts looping background music, replacing the current track."),
            e("StopMusic", ":()", "Stops the background music."),
        ],
        instance_members: &[],
    },
    Library {
        name: "Physics",
        doc: "World queries.",
        members: &[
            e("Raycast", ":(origin: Vector3, direction: Vector3, ignore: {Instance}?): RaycastResult?", "Casts a ray of length |direction|. The result has `Instance`, `Position`, `Normal` and `Distance`."),
            e("OverlapSphere", ":(center: Vector3, radius: number): {Instance}", "Parts and characters overlapping a sphere."),
        ],
        instance_members: &[],
    },
    Library {
        name: "Events",
        doc: "A message bus for scripts to talk to each other.",
        members: &[
            e("Fire", ":(name: string, ...: any)", "Sends a message to every listener of `name`."),
            e("Get", ":(name: string): Signal", "The event for `name`; use `:Connect(fn)` or `:Wait()`."),
        ],
        instance_members: &[],
    },
    Library {
        name: "Save",
        doc: "Game progress stored on this device (numbers, strings, booleans and plain tables).",
        members: &[e("Get", ":(key: string, default: any?): any", "Reads a saved value."), e("Set", ":(key: string, value: any)", "Saves a value; nil removes it.")],
        instance_members: &[],
    },
    Library {
        name: "Signal",
        doc: "Events. Engine events and `Signal.new()` share the same methods.",
        members: &[e("new", "(): Signal", "Creates an event scripts can fire themselves.")],
        instance_members: &[
            e("Connect", "(handler: function): Connection", "Calls `handler` every time the event fires. The connection has `:Disconnect()`."),
            e("Once", "(handler: function): Connection", "Calls `handler` the next time the event fires, then disconnects."),
            e("Wait", "(): ...any", "Pauses this thread until the event fires and returns its arguments."),
            e("Fire", "(...: any)", "Fires a custom signal."),
            e("DisconnectAll", "()", "Removes every listener."),
        ],
    },
    Library {
        name: "Enum",
        doc: "Named option lists: `Enum.Material.Neon`, `Enum.Shape.Sphere`, `Enum.EasingStyle.Bounce`, `Enum.KeyCode.Space`, ... Each value is its own name as a string.",
        members: &[
            e("Material", "", "Part materials."),
            e("Shape", "", "Part shapes."),
            e("CameraMode", "", "Camera modes."),
            e("TextAlign", "", "Text alignment."),
            e("CharacterState", "", "Character states."),
            e("EasingStyle", "", "Tween easing styles."),
            e("EasingDirection", "", "Tween easing directions."),
            e("KeyCode", "", "Key names."),
        ],
        instance_members: &[],
    },
    Library {
        name: "math",
        doc: "Mathematics.",
        members: &[
            e("abs", "(x: number): number", "Absolute value."),
            e("floor", "(x: number): number", "Rounds down."),
            e("ceil", "(x: number): number", "Rounds up."),
            e("round", "(x: number): number", "Rounds to the nearest whole number."),
            e("sqrt", "(x: number): number", "Square root."),
            e("sin", "(x: number): number", "Sine of an angle in radians."),
            e("cos", "(x: number): number", "Cosine of an angle in radians."),
            e("tan", "(x: number): number", "Tangent of an angle in radians."),
            e("asin", "(x: number): number", "Inverse sine."),
            e("acos", "(x: number): number", "Inverse cosine."),
            e("atan", "(y: number, x: number?): number", "Inverse tangent."),
            e("atan2", "(y: number, x: number): number", "Angle of the point (x, y)."),
            e("exp", "(x: number): number", "e raised to x."),
            e("log", "(x: number, base: number?): number", "Logarithm."),
            e("pow", "(x: number, y: number): number", "x raised to y."),
            e("fmod", "(x: number, y: number): number", "Remainder of x / y."),
            e("modf", "(x: number): (number, number)", "Whole and fractional parts."),
            e("deg", "(radians: number): number", "Radians to degrees."),
            e("rad", "(degrees: number): number", "Degrees to radians."),
            e("max", "(...: number): number", "The largest argument."),
            e("min", "(...: number): number", "The smallest argument."),
            e("clamp", "(x: number, min: number, max: number): number", "Limits x to a range."),
            e("sign", "(x: number): number", "-1, 0 or 1."),
            e("lerp", "(a: number, b: number, t: number): number", "Blends from a to b."),
            e("noise", "(x: number, y: number?, z: number?): number", "Smooth Perlin noise between -1 and 1."),
            e("random", "(m: number?, n: number?): number", "A random number: 0–1, 1–m, or m–n."),
            e("randomseed", "(seed: number)", "Seeds the random generator."),
            e("tointeger", "(x: any): number?", "Converts to an integer if exact."),
            e("pi", "", "3.14159…"),
            e("huge", "", "Infinity."),
        ],
        instance_members: &[],
    },
    Library {
        name: "string",
        doc: "Text. Also available as methods: `(\"hi\"):upper()`.",
        members: &[
            e("len", "(s: string): number", "Length in bytes."),
            e("sub", "(s: string, i: number, j: number?): string", "A substring."),
            e("upper", "(s: string): string", "Upper case."),
            e("lower", "(s: string): string", "Lower case."),
            e("reverse", "(s: string): string", "Reversed."),
            e("rep", "(s: string, n: number, sep: string?): string", "Repeated n times."),
            e("byte", "(s: string, i: number?, j: number?): ...number", "Character codes."),
            e("char", "(...: number): string", "Text from character codes."),
            e("format", "(format: string, ...: any): string", "printf-style formatting: `%d`, `%.2f`, `%s`, ..."),
            e("find", "(s: string, pattern: string, init: number?, plain: boolean?): (number?, number?)", "Finds a pattern."),
            e("match", "(s: string, pattern: string, init: number?): ...string", "Captures from the first match."),
            e("gmatch", "(s: string, pattern: string)", "Iterates over every match."),
            e("gsub", "(s: string, pattern: string, replacement: any, n: number?): (string, number)", "Replaces matches."),
            e("split", "(s: string, separator: string?): {string}", "Splits into an array."),
        ],
        instance_members: &[],
    },
    Library {
        name: "table",
        doc: "Arrays and tables.",
        members: &[
            e("insert", "(t: table, pos: number | any, value: any?)", "Appends, or inserts at a position."),
            e("remove", "(t: table, pos: number?): any", "Removes and returns an element (the last by default)."),
            e("concat", "(t: table, sep: string?, i: number?, j: number?): string", "Joins elements into text."),
            e("sort", "(t: table, comp: function?)", "Sorts in place."),
            e("unpack", "(t: table, i: number?, j: number?): ...any", "Returns the elements."),
            e("pack", "(...: any): table", "Packs arguments into a table with field `n`."),
            e("move", "(a: table, f: number, e: number, t: number, b: table?): table", "Copies a range of elements."),
            e("find", "(t: table, value: any, init: number?): number?", "Index of a value in an array."),
            e("clear", "(t: table)", "Removes every entry."),
            e("clone", "(t: table): table", "A shallow copy."),
            e("create", "(count: number, value: any?): table", "An array of `count` copies of `value`."),
        ],
        instance_members: &[],
    },
    Library {
        name: "os",
        doc: "Time and date. File and process access is not available to game scripts.",
        members: &[
            e("time", "(date: table?): number", "Seconds since 1970."),
            e("clock", "(): number", "Processor time in seconds, for measuring durations."),
            e("date", "(format: string?, time: number?): string | table", "Formats a time."),
            e("difftime", "(t2: number, t1: number): number", "t2 - t1."),
        ],
        instance_members: &[],
    },
    Library {
        name: "coroutine",
        doc: "Lua coroutines. Prefer `task` for game logic.",
        members: &[
            e("create", "(f: function): thread", "Creates a coroutine."),
            e("resume", "(co: thread, ...: any): (boolean, ...any)", "Runs a coroutine until it yields."),
            e("yield", "(...: any): ...any", "Pauses the running coroutine."),
            e("status", "(co: thread): string", "running, suspended, normal or dead."),
            e("wrap", "(f: function): function", "A function that resumes a new coroutine."),
            e("isyieldable", "(): boolean", "Whether the running code can yield."),
            e("running", "(): thread", "The running coroutine."),
            e("close", "(co: thread): boolean", "Closes a suspended coroutine."),
        ],
        instance_members: &[],
    },
    Library {
        name: "utf8",
        doc: "Unicode text helpers.",
        members: &[e("char", "(...: number): string", "Text from code points."), e("len", "(s: string): number?", "Length in characters."), e("codepoint", "(s: string, i: number?, j: number?): ...number", "Code points of characters.")],
        instance_members: &[],
    },
];

pub fn library(name: &str) -> Option<&'static Library> {
    LIBRARIES.iter().find(|l| l.name == name)
}

/// The class a service global refers to.
pub fn global_class(name: &str) -> Option<&'static ClassInfo> {
    let class = match name {
        "workspace" => "Workspace",
        "game" => "Game",
        "script" => "Script",
        "Workspace" | "Players" | "Lighting" | "Storage" | "Gui" | "Scripts" | "Camera" => name,
        _ => return None,
    };
    schema().find(class)
}

/// What kind of member a completion or reference row describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Global,
    Library,
    Function,
    Property,
    Method,
    Event,
    Class,
    Value,
    Local,
}

/// A member of an instance class, with inherited members included.
pub struct Member {
    pub name: &'static str,
    pub kind: Kind,
    pub signature: String,
    pub doc: &'static str,
}

pub fn prop_type_label(ty: &PropType) -> String {
    match ty {
        PropType::Bool => "boolean".into(),
        PropType::Number { .. } => "number".into(),
        PropType::String | PropType::Asset(_) => "string".into(),
        PropType::Vec2 => "Vector2".into(),
        PropType::Vec3 | PropType::Rotation => "Vector3".into(),
        PropType::Color => "Color3".into(),
        PropType::UDim2 => "UDim2".into(),
        PropType::Ref => "Instance?".into(),
        PropType::Enum(options) => options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(" | "),
    }
}

/// Properties, methods and events of a class (most derived first).
pub fn class_members(class: &ClassInfo) -> Vec<Member> {
    let mut members = vec![
        Member { name: "Name", kind: Kind::Property, signature: "string".into(), doc: "The instance's name." },
        Member { name: "Parent", kind: Kind::Property, signature: "Instance?".into(), doc: "The instance this one is inside. Setting it to nil removes the instance from the game without destroying it." },
        Member { name: "ClassName", kind: Kind::Property, signature: "string".into(), doc: "The class of the instance (read-only)." },
    ];
    if class.prop("Rotation").is_some() {
        members.push(Member { name: "CFrame", kind: Kind::Property, signature: "CFrame".into(), doc: "Position and orientation together." });
        members.push(Member { name: "Orientation", kind: Kind::Property, signature: "Vector3".into(), doc: "Alias of Rotation: X/Y/Z angles in degrees." });
    }
    for prop in class.props.iter().rev() {
        members.push(Member { name: prop.name, kind: Kind::Property, signature: prop_type_label(&prop.ty), doc: prop.doc });
    }
    for method in class.all_methods() {
        members.push(Member { name: method.name, kind: Kind::Method, signature: method.signature.to_string(), doc: method.doc });
    }
    for event in class.all_events() {
        members.push(Member { name: event.name, kind: Kind::Event, signature: event.signature.to_string(), doc: event.doc });
    }
    members
}

/// Renders the whole reference as Markdown (`docs/lua-api.md`).
pub fn markdown() -> String {
    let mut out = String::from(
        "# Araria Lua API\n\n<!-- Generated from the engine's class schema and crates/studio/src/code/api.rs.\n     Regenerate with: cargo test -p araria-studio -- --ignored write_api_reference -->\n\n\
Scripts are written in Lua 5.4. Each `Script` runs in its own environment and its own thread; the same\nreference is available inside the studio (the **API** tab next to the terminal) and drives autocomplete.\n\n",
    );
    out.push_str("## Contents\n\n- [Globals](#globals)\n- [Libraries](#libraries)\n- [Classes](#classes)\n\n## Globals\n\n| Name | Description |\n|---|---|\n");
    for entry in GLOBALS {
        out.push_str(&format!("| `{}{}` | {} |\n", entry.name, entry.signature.replace('|', "\\|"), entry.doc.replace('|', "\\|")));
    }
    out.push_str("\n## Libraries\n");
    for lib in LIBRARIES {
        out.push_str(&format!("\n### {}\n\n{}\n\n| Member | Description |\n|---|---|\n", lib.name, lib.doc));
        for entry in lib.members {
            let (separator, signature) = match entry.signature.strip_prefix(':') {
                Some(rest) => (":", rest),
                None => (".", entry.signature),
            };
            out.push_str(&format!("| `{}{separator}{}{}` | {} |\n", lib.name, entry.name, signature.replace('|', "\\|"), entry.doc.replace('|', "\\|")));
        }
        if !lib.instance_members.is_empty() {
            out.push_str(&format!("\nOn a {} value:\n\n| Member | Description |\n|---|---|\n", lib.name));
            for entry in lib.instance_members {
                out.push_str(&format!("| `{}{}` | {} |\n", entry.name, entry.signature.replace('|', "\\|"), entry.doc.replace('|', "\\|")));
            }
        }
    }
    out.push_str("\n## Classes\n");
    for class in schema().classes() {
        let def = &class.def;
        out.push_str(&format!("\n### {}\n\n{}", def.name, def.doc));
        if let Some(parent) = def.parent {
            out.push_str(&format!(" Inherits from [{parent}](#{}).", parent.to_lowercase()));
        }
        if def.service {
            out.push_str(" *Service: exists once in every game.*");
        } else if def.creatable {
            out.push_str(&format!(" Create with `Instance.new(\"{}\")`.", def.name));
        }
        out.push('\n');
        if !def.props.is_empty() {
            out.push_str("\n| Property | Type | Description |\n|---|---|---|\n");
            for prop in &def.props {
                let note = if prop.read_only { " *(read-only)*" } else { "" };
                out.push_str(&format!("| `{}` | {} | {}{note} |\n", prop.name, prop_type_label(&prop.ty).replace('|', "\\|"), prop.doc.replace('|', "\\|")));
            }
        }
        if !def.methods.is_empty() {
            out.push_str("\n| Method | Description |\n|---|---|\n");
            for method in &def.methods {
                out.push_str(&format!("| `:{}{}` | {} |\n", method.name, method.signature.replace('|', "\\|"), method.doc.replace('|', "\\|")));
            }
        }
        if !def.events.is_empty() {
            out.push_str("\n| Event | Description |\n|---|---|\n");
            for event in &def.events {
                out.push_str(&format!("| `{}{}` | {} |\n", event.name, event.signature.replace('|', "\\|"), event.doc.replace('|', "\\|")));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_covers_classes_and_libraries() {
        let text = markdown();
        for expected in ["### Part", "`Instance.new(className: string", "`Audio:Play(asset: string", "| `Transparency` | number |", "`:GetChildren(): {Instance}`", "### task"] {
            assert!(text.contains(expected), "missing {expected}");
        }
        assert!(global_class("workspace").is_some_and(|c| c.name() == "Workspace"));
        let part = schema().find("Part").unwrap();
        let members = class_members(part);
        assert!(members.iter().any(|m| m.name == "Touched" && m.kind == Kind::Event));
        assert!(members.iter().any(|m| m.name == "Destroy" && m.kind == Kind::Method));
        assert!(members.iter().any(|m| m.name == "Shape" && m.signature.contains("\"Sphere\"")));
    }

    /// Writes `docs/lua-api.md`. Run explicitly after changing the API.
    #[test]
    #[ignore = "writes a file in the repository"]
    fn write_api_reference() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lua-api.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, markdown()).unwrap();
    }
}
