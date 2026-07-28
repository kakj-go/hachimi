use super::*;

pub(super) fn read_json_chunk(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("invalid_glb");
    }
    if u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| "invalid_glb")?) != 2 {
        return Err("unsupported_glb_version");
    }
    let declared = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| "invalid_glb")?);
    if usize::try_from(declared).map_err(|_| "invalid_glb")? != bytes.len() {
        return Err("glb_length_mismatch");
    }
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| "invalid_glb")?);
    let chunk_type = u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| "invalid_glb")?);
    let end = 20_usize
        .checked_add(usize::try_from(json_length).map_err(|_| "invalid_glb")?)
        .ok_or("invalid_glb")?;
    if chunk_type != 0x4E4F_534A || end > bytes.len() {
        return Err("invalid_glb");
    }
    serde_json::from_slice(&bytes[20..end]).map_err(|_| "invalid_glb_json")
}

pub(super) fn reject_external_resources(json: &Value) -> Result<(), &'static str> {
    for collection in ["buffers", "images"] {
        for item in json
            .get(collection)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if item
                .get("uri")
                .and_then(Value::as_str)
                .is_some_and(|uri| !uri.starts_with("data:"))
            {
                return Err("external_resource");
            }
        }
    }
    Ok(())
}

pub(super) fn reject_unsupported_extensions(json: &Value) -> Result<(), &'static str> {
    const SUPPORTED_REQUIRED: [&str; 8] = [
        "VRM",
        "VRMC_vrm",
        "VRMC_materials_mtoon",
        "VRMC_springBone",
        "VRMC_node_constraint",
        "KHR_materials_unlit",
        "KHR_texture_transform",
        "KHR_mesh_quantization",
    ];
    if json
        .get("extensionsRequired")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|extension| !SUPPORTED_REQUIRED.contains(&extension))
    {
        return Err("unsupported_required_extension");
    }
    Ok(())
}

pub(super) fn validate_embedded_resource_bounds(
    bytes: &[u8],
    json: &Value,
) -> Result<(), &'static str> {
    let binary_length = glb_binary_chunk_length(bytes)?;
    let buffers = json
        .get("buffers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, buffer) in buffers.iter().enumerate() {
        let declared = buffer
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        if buffer.get("uri").is_none() && (index != 0 || declared > binary_length as u64) {
            return Err("resource_out_of_bounds");
        }
    }
    for view in json
        .get("bufferViews")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let buffer_index = view
            .get("buffer")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("resource_out_of_bounds")?;
        let buffer_length = buffers
            .get(buffer_index)
            .and_then(|buffer| buffer.get("byteLength"))
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
        let length = view
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > buffer_length)
        {
            return Err("resource_out_of_bounds");
        }
    }
    Ok(())
}

pub(super) fn glb_binary_chunk_length(bytes: &[u8]) -> Result<usize, &'static str> {
    let json_length = u32::from_le_bytes(
        bytes
            .get(12..16)
            .ok_or("invalid_glb")?
            .try_into()
            .map_err(|_| "invalid_glb")?,
    );
    let mut cursor = 20_usize
        .checked_add(usize::try_from(json_length).map_err(|_| "invalid_glb")?)
        .ok_or("invalid_glb")?;
    while cursor < bytes.len() {
        let header_end = cursor.checked_add(8).ok_or("invalid_glb")?;
        let header = bytes.get(cursor..header_end).ok_or("invalid_glb")?;
        let length = usize::try_from(u32::from_le_bytes(
            header[0..4].try_into().map_err(|_| "invalid_glb")?,
        ))
        .map_err(|_| "invalid_glb")?;
        let kind = u32::from_le_bytes(header[4..8].try_into().map_err(|_| "invalid_glb")?);
        let end = header_end.checked_add(length).ok_or("invalid_glb")?;
        if end > bytes.len() {
            return Err("resource_out_of_bounds");
        }
        if kind == 0x004E_4942 {
            return Ok(length);
        }
        cursor = end;
    }
    Ok(0)
}

pub(super) fn reachable_scene_content(gltf: &Gltf) -> (BTreeSet<usize>, BTreeSet<usize>) {
    fn visit(node: gltf::Node<'_>, nodes: &mut BTreeSet<usize>, meshes: &mut BTreeSet<usize>) {
        if !nodes.insert(node.index()) {
            return;
        }
        if let Some(mesh) = node.mesh() {
            meshes.insert(mesh.index());
        }
        for child in node.children() {
            visit(child, nodes, meshes);
        }
    }

    let mut nodes = BTreeSet::new();
    let mut meshes = BTreeSet::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            visit(node, &mut nodes, &mut meshes);
        }
    }
    (nodes, meshes)
}

pub(super) fn accessor_bounds(json: &Value, index: usize) -> Option<([f64; 3], [f64; 3])> {
    let accessor = json.get("accessors")?.as_array()?.get(index)?;
    let minimum = vec3(accessor.get("min")?)?;
    let maximum = vec3(accessor.get("max")?)?;
    minimum
        .iter()
        .chain(maximum.iter())
        .all(|value| value.is_finite())
        .then_some((minimum, maximum))
}

pub(super) fn vec3(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    (values.len() == 3).then(|| {
        [
            values[0].as_f64()?,
            values[1].as_f64()?,
            values[2].as_f64()?,
        ]
        .into()
    })?
}

pub(super) fn detect_format(json: &Value) -> AvatarFormat {
    let extensions = json.get("extensions").and_then(Value::as_object);
    if extensions.is_some_and(|value| value.contains_key("VRMC_vrm")) {
        AvatarFormat::Vrm1
    } else if extensions.is_some_and(|value| value.contains_key("VRM")) {
        AvatarFormat::Vrm0
    } else {
        AvatarFormat::Glb
    }
}

pub(super) fn explicit_humanoid_bones(json: &Value, format: AvatarFormat) -> BTreeMap<String, u32> {
    let mut bones = BTreeMap::new();
    match format {
        AvatarFormat::Vrm1 => {
            if let Some(values) = json
                .pointer("/extensions/VRMC_vrm/humanoid/humanBones")
                .and_then(Value::as_object)
            {
                for (bone, binding) in values {
                    if let Some(node) = binding.get("node").and_then(Value::as_u64)
                        && let Ok(node) = u32::try_from(node)
                    {
                        bones.insert(canonical_bone_name(bone), node);
                    }
                }
            }
        }
        AvatarFormat::Vrm0 => {
            for binding in json
                .pointer("/extensions/VRM/humanoid/humanBones")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(bone), Some(node)) = (
                    binding.get("bone").and_then(Value::as_str),
                    binding.get("node").and_then(Value::as_u64),
                ) && let Ok(node) = u32::try_from(node)
                {
                    bones.insert(canonical_bone_name(bone), node);
                }
            }
        }
        AvatarFormat::Glb => {}
    }
    if let Some(values) = json
        .pointer("/asset/extras/hachimiAvatar/humanoid")
        .and_then(Value::as_object)
    {
        for (bone, binding) in values {
            let node = binding.as_u64().or_else(|| binding.get("node")?.as_u64());
            if let Some(node) = node.and_then(|value| u32::try_from(value).ok()) {
                bones.insert(canonical_bone_name(bone), node);
            }
        }
    }
    bones
}

pub(super) fn node_world_positions(
    gltf: &Gltf,
    parents: &[Option<usize>],
) -> Vec<Option<[f32; 3]>> {
    let local: Vec<_> = gltf.nodes().map(|node| node.transform().matrix()).collect();
    (0..local.len())
        .map(|index| {
            let mut chain = Vec::new();
            let mut cursor = Some(index);
            let mut remaining = local.len();
            while let Some(node) = cursor {
                if remaining == 0 || chain.contains(&node) {
                    return None;
                }
                chain.push(node);
                cursor = parents.get(node).copied().flatten();
                remaining -= 1;
            }
            let world = chain
                .into_iter()
                .rev()
                .fold(identity_matrix(), |parent, node| {
                    multiply_matrices(parent, local[node])
                });
            let position = [world[3][0], world[3][1], world[3][2]];
            position
                .iter()
                .all(|value| value.is_finite())
                .then_some(position)
        })
        .collect()
}

const fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub(super) fn multiply_matrices(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4)
                .map(|index| left[index][row] * right[column][index])
                .sum();
        }
    }
    result
}

pub(super) fn parent_indices(json: &Value, node_count: usize) -> Vec<Option<usize>> {
    let mut parents = vec![None; node_count];
    for (parent, node) in json
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter_map(|value| usize::try_from(value).ok())
        {
            if child < parents.len() {
                parents[child] = Some(parent);
            }
        }
    }
    parents
}

pub(super) fn is_descendant(mut child: usize, ancestor: usize, parents: &[Option<usize>]) -> bool {
    let mut remaining = parents.len();
    while remaining > 0 {
        if child == ancestor {
            return true;
        }
        let Some(parent) = parents.get(child).copied().flatten() else {
            return false;
        };
        child = parent;
        remaining -= 1;
    }
    false
}

pub(super) fn expression_bindings(
    json: &Value,
    format: AvatarFormat,
) -> Vec<AvatarExpressionBinding> {
    let mut bindings = standard_expression_bindings(json, format);
    bindings.extend(generic_expression_bindings(json));
    bindings
}

pub(super) fn standard_expression_bindings(
    json: &Value,
    format: AvatarFormat,
) -> Vec<AvatarExpressionBinding> {
    let mut bindings = Vec::new();
    if format == AvatarFormat::Vrm1 {
        if let Some(presets) = json
            .pointer("/extensions/VRMC_vrm/expressions/preset")
            .and_then(Value::as_object)
        {
            for (name, expression) in presets {
                let expression_name = canonical_expression_name(name);
                for binding in expression
                    .get("morphTargetBinds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    push_expression_binding(&mut bindings, &expression_name, binding, "node");
                }
            }
        }
    } else if format == AvatarFormat::Vrm0 {
        let mesh_nodes = mesh_node_indices(json);
        for group in json
            .pointer("/extensions/VRM/blendShapeMaster/blendShapeGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let preset = group.get("presetName").and_then(Value::as_str);
            let Some(name) = preset
                .filter(|name| !name.eq_ignore_ascii_case("unknown") && !name.is_empty())
                .or_else(|| group.get("name").and_then(Value::as_str))
            else {
                continue;
            };
            let expression_name = canonical_expression_name(name);
            for binding in group
                .get("binds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(mesh) = binding.get("mesh").and_then(Value::as_u64) else {
                    continue;
                };
                let Some(node) = mesh_nodes.get(&(mesh as usize)).copied() else {
                    continue;
                };
                if let Some(index) = binding.get("index").and_then(Value::as_u64)
                    && let (Ok(node_index), Ok(morph_index)) =
                        (u32::try_from(node), u32::try_from(index))
                {
                    bindings.push(AvatarExpressionBinding {
                        expression: expression_name.clone(),
                        node_index,
                        morph_index,
                    });
                }
            }
        }
    }
    bindings
}

pub(super) fn declared_standard_expression_names(
    json: &Value,
    format: AvatarFormat,
) -> BTreeSet<String> {
    match format {
        AvatarFormat::Vrm1 => json
            .pointer("/extensions/VRMC_vrm/expressions/preset")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|presets| presets.keys())
            .map(|name| canonical_expression_name(name))
            .collect(),
        AvatarFormat::Vrm0 => json
            .pointer("/extensions/VRM/blendShapeMaster/blendShapeGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|group| {
                let preset = group.get("presetName").and_then(Value::as_str);
                preset
                    .filter(|name| !name.eq_ignore_ascii_case("unknown") && !name.is_empty())
                    .or_else(|| group.get("name").and_then(Value::as_str))
            })
            .map(canonical_expression_name)
            .collect(),
        AvatarFormat::Glb => BTreeSet::new(),
    }
}

pub(super) fn generic_expression_bindings(json: &Value) -> Vec<AvatarExpressionBinding> {
    let mut result = Vec::new();
    let mesh_nodes = mesh_node_indices(json);
    for (mesh_index, mesh) in json
        .get("meshes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let names = mesh
            .pointer("/extras/targetNames")
            .and_then(Value::as_array)
            .or_else(|| mesh.get("targetNames").and_then(Value::as_array));
        let Some(node_index) = mesh_nodes.get(&mesh_index).copied() else {
            continue;
        };
        for (morph_index, name) in names.into_iter().flatten().enumerate() {
            let Some(name) = name.as_str() else { continue };
            let expression = canonical_expression_name(name);
            if is_known_expression(&expression) {
                result.push(AvatarExpressionBinding {
                    expression,
                    node_index: to_u32(node_index),
                    morph_index: to_u32(morph_index),
                });
            }
        }
    }
    result
}

pub(super) fn push_expression_binding(
    output: &mut Vec<AvatarExpressionBinding>,
    expression: &str,
    binding: &Value,
    node_key: &str,
) {
    let Some(node) = binding.get(node_key).and_then(Value::as_u64) else {
        return;
    };
    let Some(index) = binding.get("index").and_then(Value::as_u64) else {
        return;
    };
    if let (Ok(node_index), Ok(morph_index)) = (u32::try_from(node), u32::try_from(index)) {
        output.push(AvatarExpressionBinding {
            expression: expression.to_owned(),
            node_index,
            morph_index,
        });
    }
}

pub(super) fn mesh_node_indices(json: &Value) -> BTreeMap<usize, usize> {
    json.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(node_index, node)| {
            Some((
                usize::try_from(node.get("mesh")?.as_u64()?).ok()?,
                node_index,
            ))
        })
        .collect()
}

pub(super) fn has_path(json: &Value, path: &[&str]) -> bool {
    path.iter()
        .try_fold(json, |value, key| value.get(*key))
        .is_some()
}

pub(super) fn canonical_bone_name(name: &str) -> String {
    let normalized = normalize_name(name);
    let aliases = [
        ("upperchest", "upper_chest"),
        ("leftshoulder", "left_shoulder"),
        ("leftupperarm", "left_upper_arm"),
        ("leftlowerarm", "left_lower_arm"),
        ("lefthand", "left_hand"),
        ("rightupperarm", "right_upper_arm"),
        ("rightlowerarm", "right_lower_arm"),
        ("righthand", "right_hand"),
        ("leftupperleg", "left_upper_leg"),
        ("leftlowerleg", "left_lower_leg"),
        ("leftfoot", "left_foot"),
        ("lefttoes", "left_toes"),
        ("lefteye", "left_eye"),
        ("rightshoulder", "right_shoulder"),
        ("rightupperleg", "right_upper_leg"),
        ("rightlowerleg", "right_lower_leg"),
        ("rightfoot", "right_foot"),
        ("righttoes", "right_toes"),
        ("righteye", "right_eye"),
        ("leftthumbproximal", "left_thumb_proximal"),
        ("leftthumbintermediate", "left_thumb_intermediate"),
        ("leftthumbdistal", "left_thumb_distal"),
        ("leftindexproximal", "left_index_proximal"),
        ("leftindexintermediate", "left_index_intermediate"),
        ("leftindexdistal", "left_index_distal"),
        ("leftmiddleproximal", "left_middle_proximal"),
        ("leftmiddleintermediate", "left_middle_intermediate"),
        ("leftmiddledistal", "left_middle_distal"),
        ("leftringproximal", "left_ring_proximal"),
        ("leftringintermediate", "left_ring_intermediate"),
        ("leftringdistal", "left_ring_distal"),
        ("leftlittleproximal", "left_little_proximal"),
        ("leftlittleintermediate", "left_little_intermediate"),
        ("leftlittledistal", "left_little_distal"),
        ("rightthumbproximal", "right_thumb_proximal"),
        ("rightthumbintermediate", "right_thumb_intermediate"),
        ("rightthumbdistal", "right_thumb_distal"),
        ("rightindexproximal", "right_index_proximal"),
        ("rightindexintermediate", "right_index_intermediate"),
        ("rightindexdistal", "right_index_distal"),
        ("rightmiddleproximal", "right_middle_proximal"),
        ("rightmiddleintermediate", "right_middle_intermediate"),
        ("rightmiddledistal", "right_middle_distal"),
        ("rightringproximal", "right_ring_proximal"),
        ("rightringintermediate", "right_ring_intermediate"),
        ("rightringdistal", "right_ring_distal"),
        ("rightlittleproximal", "right_little_proximal"),
        ("rightlittleintermediate", "right_little_intermediate"),
        ("rightlittledistal", "right_little_distal"),
    ];
    aliases
        .iter()
        .find_map(|(alias, canonical)| (normalized == *alias).then_some(*canonical))
        .unwrap_or(name)
        .to_owned()
}

pub(super) fn canonical_expression_name(name: &str) -> String {
    match normalize_name(name).as_str() {
        "a" | "aa" | "visemeaa" | "moutha" => "aa",
        "i" | "ih" | "visemeih" | "mouthi" => "ih",
        "u" | "ou" | "visemeou" | "mouthu" => "ou",
        "e" | "ee" | "visemeee" | "mouthe" => "ee",
        "o" | "oh" | "visemeoh" | "moutho" => "oh",
        "blink" | "blinkboth" => "blink",
        "blinkleft" | "blinkl" => "blink_left",
        "blinkright" | "blinkr" => "blink_right",
        "neutral" => "neutral",
        "joy" | "happy" => "happy",
        "fun" | "relaxed" => "relaxed",
        "sorrow" | "sad" => "sad",
        "angry" => "angry",
        "surprise" | "surprised" => "surprised",
        other => other,
    }
    .to_owned()
}

pub(super) fn is_known_expression(name: &str) -> bool {
    matches!(
        name,
        "aa" | "ih"
            | "ou"
            | "ee"
            | "oh"
            | "blink"
            | "blink_left"
            | "blink_right"
            | "neutral"
            | "happy"
            | "relaxed"
            | "sad"
            | "angry"
            | "surprised"
    )
}

pub(super) fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn validate_name(name: &str) -> Result<String, AvatarError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AvatarError::InvalidName);
    }
    Ok(name.to_owned())
}

pub(super) fn incompatible_assessment(code: &str) -> AvatarAssessment {
    AvatarAssessment {
        compatibility: AvatarCompatibility::Incompatible,
        detector_version: DETECTOR_VERSION,
        capabilities: Vec::new(),
        statistics: AvatarStatistics::default(),
        requirements: Vec::new(),
        issues: vec![issue(code, AvatarIssueSeverity::Error)],
    }
}

pub(super) fn issue(code: &str, severity: AvatarIssueSeverity) -> AvatarIssue {
    AvatarIssue {
        code: code.to_owned(),
        severity,
    }
}

pub(super) fn newest_compatible_id(entries: &[AvatarEntry]) -> Option<String> {
    entries
        .iter()
        .filter(|entry| entry.assessment.compatibility == AvatarCompatibility::RuntimeReady)
        .max_by_key(|entry| entry.imported_at.parse::<u128>().unwrap_or_default())
        .map(|entry| entry.id.clone())
}

pub(super) fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

pub(super) fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
