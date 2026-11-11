#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

/// One mesh of a fixture: its vertices, its triangles as indices, and the
/// material index of its one primitive.
#[derive(Clone)]
pub struct Mesh {
    pub points: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

/// One node of a fixture, placing a mesh with an optional transform, as
/// glTF JSON fields.
#[derive(Clone)]
pub struct Node {
    pub mesh: usize,
    pub transform: String,
}

pub fn node(mesh: usize) -> Node {
    Node {
        mesh,
        transform: String::new(),
    }
}

#[derive(Clone)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Option<&'static str>>,
    pub nodes: Vec<Node>,
}

pub fn one(mesh: Mesh, materials: Vec<Option<&'static str>>) -> Scene {
    Scene {
        meshes: vec![mesh],
        materials,
        nodes: vec![node(0)],
    }
}

/// The JSON chunk and the binary chunk of `scene`: per mesh, its
/// positions and then its indices, each in a buffer view of its own.
pub fn chunks(scene: &Scene) -> (String, Vec<u8>) {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    for mesh in &scene.meshes {
        let from = bin.len();
        for point in &mesh.points {
            for value in point {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{from},"byteLength":{}}}"#,
            bin.len() - from
        ));
        let low: Vec<String> = (0..3)
            .map(|axis| {
                let least = mesh
                    .points
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min);
                format!("{least:?}")
            })
            .collect();
        let high: Vec<String> = (0..3)
            .map(|axis| {
                let most = mesh
                    .points
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max);
                format!("{most:?}")
            })
            .collect();
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5126,"count":{},"type":"VEC3","min":[{}],"max":[{}]}}"#,
            views.len() - 1,
            mesh.points.len(),
            low.join(","),
            high.join(",")
        ));
        let positions = accessors.len() - 1;
        let from = bin.len();
        for index in &mesh.indices {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{from},"byteLength":{}}}"#,
            bin.len() - from
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5125,"count":{},"type":"SCALAR"}}"#,
            views.len() - 1,
            mesh.indices.len()
        ));
        let material = mesh
            .material
            .map(|at| format!(r#","material":{at}"#))
            .unwrap_or_default();
        meshes.push(format!(
            r#"{{"primitives":[{{"attributes":{{"POSITION":{positions}}},"indices":{}{material}}}]}}"#,
            accessors.len() - 1
        ));
    }
    let materials: Vec<String> = scene
        .materials
        .iter()
        .map(|name| match name {
            Some(name) => format!(r#"{{"name":"{name}"}}"#),
            None => "{}".to_owned(),
        })
        .collect();
    let nodes: Vec<String> = scene
        .nodes
        .iter()
        .map(|node| format!(r#"{{"mesh":{}{}}}"#, node.mesh, node.transform))
        .collect();
    let roots: Vec<String> = (0..scene.nodes.len()).map(|at| at.to_string()).collect();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{}]}}],"nodes":[{}],"meshes":[{}],"materials":[{}],"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]}}"#,
        roots.join(","),
        nodes.join(","),
        meshes.join(","),
        materials.join(","),
        accessors.join(","),
        views.join(","),
        bin.len()
    );
    (json, bin)
}

/// A binary glTF file of a JSON chunk and a binary chunk, each padded to
/// four bytes as the format asks.
pub fn pack(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2_u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

pub fn glb(scene: &Scene) -> Vec<u8> {
    let (json, bin) = chunks(scene);
    pack(&json, &bin)
}

/// `scene` with its JSON changed by `edit`.
pub fn edited(scene: &Scene, edit: impl Fn(String) -> String) -> Vec<u8> {
    let (json, bin) = chunks(scene);
    let changed = edit(json.clone());
    assert_ne!(changed, json, "the edit changed nothing");
    pack(&changed, &bin)
}

pub fn cuboid(low: [f32; 3], high: [f32; 3], material: Option<usize>) -> Mesh {
    let points = (0..8)
        .map(|corner| {
            [0, 1, 2].map(|axis| {
                if corner >> axis & 1 == 1 {
                    high[axis]
                } else {
                    low[axis]
                }
            })
        })
        .collect();
    let faces = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let indices = faces
        .iter()
        .flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d])
        .collect();
    Mesh {
        points,
        indices,
        material,
    }
}

/// A UV sphere of `radius` about the origin, with its poles on y, and
/// without the fan of triangles around the top pole when `open`.
pub fn sphere(radius: f32, segments: u32, stacks: u32, open: bool) -> Mesh {
    let mut points = vec![[0.0, radius, 0.0]];
    for stack in 1..stacks {
        let polar = std::f32::consts::PI * stack as f32 / stacks as f32;
        for segment in 0..segments {
            let around = std::f32::consts::TAU * segment as f32 / segments as f32;
            points.push([
                radius * polar.sin() * around.cos(),
                radius * polar.cos(),
                radius * polar.sin() * around.sin(),
            ]);
        }
    }
    points.push([0.0, -radius, 0.0]);
    let ring = |stack: u32, segment: u32| 1 + (stack - 1) * segments + segment % segments;
    let bottom = points.len() as u32 - 1;
    let mut indices = Vec::new();
    for segment in 0..segments {
        if !open {
            indices.extend([0, ring(1, segment + 1), ring(1, segment)]);
        }
        for stack in 1..stacks - 1 {
            let (a, b) = (ring(stack, segment), ring(stack, segment + 1));
            let (c, d) = (ring(stack + 1, segment), ring(stack + 1, segment + 1));
            indices.extend([a, b, d, a, d, c]);
        }
        indices.extend([
            ring(stacks - 1, segment),
            ring(stacks - 1, segment + 1),
            bottom,
        ]);
    }
    Mesh {
        points,
        indices,
        material: None,
    }
}

/// A torus about the y axis: the ring of `major` radius, the tube of
/// `minor` radius.
pub fn torus(major: f32, minor: f32, around: u32, across: u32) -> Mesh {
    let mut points = Vec::new();
    for step in 0..around {
        let u = std::f32::consts::TAU * step as f32 / around as f32;
        for slice in 0..across {
            let v = std::f32::consts::TAU * slice as f32 / across as f32;
            let reach = major + minor * v.cos();
            points.push([reach * u.cos(), minor * v.sin(), reach * u.sin()]);
        }
    }
    let at = |step: u32, slice: u32| (step % around) * across + slice % across;
    let mut indices = Vec::new();
    for step in 0..around {
        for slice in 0..across {
            let (a, b) = (at(step, slice), at(step + 1, slice));
            let (c, d) = (at(step, slice + 1), at(step + 1, slice + 1));
            indices.extend([a, b, d, a, d, c]);
        }
    }
    Mesh {
        points,
        indices,
        material: None,
    }
}
