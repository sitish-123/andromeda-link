//! Generates the `org.andromeda.Link1` client from `api/org.andromeda.Link1.xml`, the API's one source.

const SOURCE: &str = "../../api/org.andromeda.Link1.xml";
const INTERFACES: [&str; 2] = ["org.andromeda.Link1", "org.andromeda.Link1.QuickShare"];
/// Kept equal to `SERVICE` and `PATH` in `src/lib.rs`.
const SERVICE: &str = "org.andromeda.Link1";
const PATH: &str = "/org/andromeda/Link1";

fn main() {
    println!("cargo::rerun-if-changed={SOURCE}");
    let xml = std::fs::read_to_string(SOURCE).expect("reading the API definition");
    let node = zbus_xml::Node::from_reader(xml.as_bytes()).expect("parsing the API definition");
    let service = zbus::names::BusName::from_static_str(SERVICE).unwrap();
    let path = zbus::ObjectPath::from_static_str(PATH).unwrap();
    let mut code = String::new();
    for name in INTERFACES {
        let interface = node.interfaces().iter().find(|interface| interface.name() == name).expect(name);
        code += &zbus_xmlgen::CodeGenerator::new()
            .with_service(Some(&service))
            .with_path(Some(&path))
            .interface_code(interface)
            .expect("generating the client");
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("link1.rs");
    std::fs::write(out, code).expect("writing the client");
}
