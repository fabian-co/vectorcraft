//! Swatch libraries: listing, reading, adding to the document (deduped, one undo step, applied)
//! and Default Swatches.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn list_and_get_every_builtin_library() {
    let mut s = Session::new();
    let libs = run(&mut s, "swatch.library.list", json!({}));
    let libs = libs["libraries"].as_array().unwrap();
    let ids: Vec<&str> = libs.iter().map(|l| l["id"].as_str().unwrap()).collect();
    for id in [
        "web-safe-216",
        "grays-neutrals",
        "earth-tones",
        "skin-tone-ramps",
        "pastels",
        "brights",
        "metallic-gradients",
        "perceptual-scales",
        "harmony-sets",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    let web = libs.iter().find(|l| l["id"] == "web-safe-216").unwrap();
    assert_eq!((web["name"].as_str(), web["count"].as_u64()), (Some("Web Safe 216"), Some(216)));
    // By id or by name in any case; groups list their swatches.
    let g = run(&mut s, "swatch.library.get", json!({"library": "earth tones"}));
    assert_eq!(g["id"], "earth-tones");
    let clay = g["groups"].as_array().unwrap().iter().find(|x| x["name"] == "Clay").unwrap();
    assert_eq!(clay["swatches"].as_array().unwrap().len(), 6);
    assert!(g["swatches"].as_array().unwrap().iter().all(|w| w["kind"] == "color" && w["group"].is_string()));
    let m = run(&mut s, "swatch.library.get", json!({"library": "metallic-gradients"}));
    assert!(m["swatches"].as_array().unwrap().iter().all(|w| w["kind"] == "gradient"));
    assert!(s.execute("swatch.library.get", &json!({"library": "nope"})).is_err());
}

#[test]
fn add_dedupes_as_one_undo_step() {
    let mut s = session();
    let before = undo_len(&s);
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Clay", "Ochre 2", "Clay 3"]}));
    // The Clay group comes whole (its third swatch once); Ochre 2 comes ungrouped.
    let added: Vec<&str> = r["added"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(added.len(), 7, "{added:?}");
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let d = doc(&s);
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Clay").map(|g| g.swatches.len()), Some(6));
    assert!(d.swatches.iter().any(|w| w.name == "Ochre 2"));
    // Adding again finds them all and changes nothing (no undo step).
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Clay", "Ochre 2"]}));
    assert_eq!((r["added"].as_array().unwrap().len(), r["existing"].as_array().unwrap().len()), (0, 7));
    assert_eq!(undo_len(&s), before + 1);
    // A name taken by a different colour gets a number.
    run(&mut s, "swatch.new", json!({"name": "Ochre 3", "color": "#123456"}));
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Ochre 3"]}));
    assert_eq!(r["added"], json!(["Ochre 3 2"]));
    // Undo removes the whole first add.
    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert!(doc(&s).swatch_groups.iter().all(|g| g.name != "Clay") && doc(&s).swatch("Ochre 2").is_none());
    assert!(s.execute("swatch.library.add", &json!({"library": "earth-tones", "names": ["Nope"]})).is_err());
}

#[test]
fn add_with_apply_paints_the_selection_in_the_same_undo_step() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
    let before = undo_len(&s);
    let r = run(&mut s, "swatch.library.add", json!({"library": "web-safe-216", "names": ["#FF6600"], "apply": "fill"}));
    assert_eq!((r["added"].clone(), r["applied"].clone()), (json!(["#FF6600"]), json!("#FF6600")));
    let fill = doc(&s).node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#ff6600".into()));
    assert_eq!(undo_len(&s), before + 1, "added and applied as one step");
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).swatch("#FF6600").is_none());
    // A gradient swatch applied to the stroke; an existing swatch is applied without an add.
    run(&mut s, "swatch.library.add", json!({"library": "metallic-gradients", "names": ["Gold"], "apply": "stroke"}));
    let n = doc(&s).node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
    assert!(matches!(n.appearance.stroke_paint(), Paint::Gradient(_)));
    let r = run(&mut s, "swatch.library.add", json!({"library": "metallic-gradients", "names": ["Gold"], "apply": "fill"}));
    assert_eq!((r["added"].clone(), r["applied"].clone()), (json!([]), json!("Gold")));
}

#[test]
fn reset_defaults_restores_missing_swatches_or_replaces_them() {
    let mut s = session();
    let defaults: Vec<String> = doc(&s).swatches_iter().map(|w| w.name.clone()).collect();
    run(&mut s, "swatch.delete", json!({"names": ["Red", "Grays"]}));
    run(&mut s, "swatch.new", json!({"name": "Mine", "color": "#abcdef"}));
    let r = run(&mut s, "swatch.resetDefaults", json!({}));
    assert!(r["added"].as_array().unwrap().iter().any(|n| n == "Red"));
    let d = doc(&s);
    assert!(d.swatch("Mine").is_some(), "the user's swatches stay");
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Grays").map(|g| g.swatches.len()), Some(9));
    assert!(defaults.iter().all(|n| d.swatch(n).is_some()));
    // Replace: exactly the defaults; art linked to a removed global swatch keeps its colour.
    run(&mut s, "swatch.edit", json!({"name": "Mine", "global": true}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "paint.setFill", json!({"swatch": "Mine"}));
    assert_eq!(s.paint.fill, Paint::Solid { color: Color::from_hex("#abcdef").unwrap(), swatch: Some("Mine".into()), tint: 1.0 });
    run(&mut s, "swatch.resetDefaults", json!({"replace": true}));
    let d = doc(&s);
    assert_eq!(d.swatches_iter().map(|w| w.name.clone()).collect::<Vec<_>>(), defaults);
    let fill = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::solid(Color::from_hex("#abcdef").unwrap()));
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#abcdef").unwrap()), "the default fill is unlinked too");
}

/// A fresh scratch folder for one test.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-swatchlib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn save_and_load_round_trip_keeps_cmyk_spot_and_groups() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": {"c": 1.0, "m": 0.5, "y": 0.0, "k": 0.2}, "spot": true}));
    run(&mut s, "swatch.newGroup", json!({"name": "Brand", "colors": ["#123456", "#abcdef"]}));
    let r = run(&mut s, "swatch.library.save", json!({"names": ["Ink", "Brand", "Sunset"], "name": "Brand Kit"}));
    assert_eq!((r["format"].as_str(), r["count"].as_u64()), (Some("vcswatches"), Some(4)));
    let data = r["data"].as_str().unwrap().to_string();
    let r = run(&mut s, "swatch.library.load", json!({"data": data, "name": "kit.vcswatches"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Brand Kit"), Some(4)));
    let id = r["library"].as_str().unwrap().to_string();
    let (_, lib) = cmd::swatchlib::library(&s, &id).unwrap();
    let ink = lib.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global);
    assert_eq!(ink.paint.color(), Some(Color::cmyk(1.0, 0.5, 0.0, 0.2)));
    assert_eq!(lib.groups[0].name, "Brand");
    assert_eq!(lib.groups[0].swatches.len(), 2);
    assert!(matches!(lib.swatch("Sunset").unwrap().paint, Paint::Gradient(_)));
    // Listed as loaded; adding from it works like any library.
    let list = run(&mut s, "swatch.library.list", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().any(|l| l["id"] == id.as_str() && l["category"] == "loaded"));
    assert!(s.execute("swatch.library.save", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("swatch.library.save", &json!({"user": true})).is_err(), "no user folder in a headless session");
}

#[test]
fn libraries_saved_in_the_user_folder_are_user_defined() {
    let dir = temp_dir("user");
    let mut s = session();
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "swatch.library.save", json!({"user": true, "name": "My: Greys", "format": "gpl", "names": ["Grays"]}));
    assert_eq!(r["library"], "user/My- Greys.gpl");
    let path = r["path"].as_str().unwrap().to_string();
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("GIMP Palette\nName: My: Greys\n"));
    let list = run(&mut s, "swatch.library.list", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("My: Greys"), Some(9)));
    // Opening a file of the user folder gives its User Defined library.
    let r = run(&mut s, "swatch.library.load", json!({ "path": path }));
    assert_eq!(r["library"], "user/My- Greys.gpl");
    // CSS by extension.
    let css = dir.join("out.css").to_string_lossy().to_string();
    run(&mut s, "swatch.library.save", json!({ "path": css }));
    assert!(std::fs::read_to_string(&css).unwrap().contains("  --white: #ffffff;"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A synthetic Lab colour book (`.acb`) of spot colours named `prefix` + each of `colors`.
fn color_book(title: &str, prefix: &str, colors: &[(&str, [u8; 3])]) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        out.extend((units.len() as u32).to_be_bytes());
        out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    }
    let mut out = b"8BCB\0\x01\x0b\xb8".to_vec();
    for s in [title, prefix, "", "A test book"] {
        string(&mut out, s);
    }
    out.extend((colors.len() as u16).to_be_bytes());
    out.extend([0, 7, 0, 1, 0, 7]);
    for (i, (name, lab)) in colors.iter().enumerate() {
        string(&mut out, name);
        out.extend(format!("{i:06}").as_bytes());
        out.extend(lab);
    }
    out.extend(b"spflspot");
    out
}

#[test]
fn colour_books_load_as_spot_libraries_and_add_to_the_document() {
    let book =
        color_book("$$$/colorbook/Test/title=Test Solid.acb", "TEST ", &[("Red 032", [128, 208, 178]), ("", [0, 0, 0]), ("Black 7", [60, 129, 130])]);
    let mut s = session();
    let b64 = vectorcraft_format::base64_encode(&book);
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": b64, "name": "whatever.acb"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Test Solid"), Some(2)), "the padding record is skipped");
    let id = r["library"].as_str().unwrap().to_string();
    let g = run(&mut s, "swatch.library.get", json!({ "library": id }));
    assert_eq!(g["swatches"][0]["name"], "TEST Red 032");
    assert_eq!((g["swatches"][0]["spot"].as_bool(), g["swatches"][0]["global"].as_bool()), (Some(true), Some(true)));
    // Adding one brings a Lab spot swatch and paints with it, linked.
    let rect = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [rect]}));
    let r = run(&mut s, "swatch.library.add", json!({"library": id, "names": ["TEST Red 032"], "apply": "fill"}));
    assert_eq!(r["added"], json!(["TEST Red 032"]));
    let w = doc(&s).swatch("TEST Red 032").unwrap();
    assert!(w.spot && w.global);
    assert!(matches!(w.paint.color(), Some(Color::Lab { a, b, .. }) if a == 80.0 && b == 50.0));
    assert!(matches!(&s.paint.fill, Paint::Solid { swatch: Some(n), .. } if n == "TEST Red 032"));
    // A cut book is an error, not a document to open.
    let cut = vectorcraft_format::base64_encode(&book[..book.len() - 20]);
    let e = s.execute("swatch.library.load", &json!({"dataBase64": cut, "name": "cut.acb"})).unwrap_err().to_string();
    assert!(e.contains("cut short"), "{e}");
    // In the user library folder it is a User Defined library, and File › Open's filters list it.
    let dir = temp_dir("acb");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Mine.acb"), &book).unwrap();
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s, "swatch.library.list", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().any(|l| l["id"] == "user/Mine.acb" && l["name"] == "Test Solid" && l["count"] == 2));
    assert!(cmd::fileio::open_filters().all(|(label, exts)| !matches!(label, "All readable files" | "Swatch libraries") || exts.contains(&"acb")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn another_documents_swatches_load_as_a_library() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Signal", "color": "#ff3300"}));
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!(r["name"], "Poster");
    let (_, lib) = cmd::swatchlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.swatch("Signal").is_some() && lib.swatch("[None]").is_none());
    assert!(s.execute("swatch.library.load", &json!({"data": "hello", "name": "x.txt"})).is_err());
}

#[test]
fn gradient_libraries_are_listed_and_gradient_swatches_rename_and_take_new_gradients() {
    let mut s = session();
    let list = run(&mut s, "swatch.library.list", json!({}));
    let grads: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "gradients").collect();
    assert!(grads.len() >= 5 && grads.iter().any(|l| l["id"] == "sky-gradients"));
    run(&mut s, "swatch.library.add", json!({"library": "sky-gradients", "names": ["Dawn"]}));
    // Rename keeps the gradient.
    let before = doc(&s).swatch("Dawn").unwrap().paint.clone();
    run(&mut s, "swatch.edit", json!({"name": "Dawn", "newName": "Early"}));
    assert_eq!(doc(&s).swatch("Early").unwrap().paint, before);
    // An edited gradient replaces it, unplaced and unlinked; one undo step.
    let undo = undo_len(&s);
    let g =
        json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}], "swatch": "Early"}});
    run(&mut s, "swatch.edit", json!({"name": "Early", "paint": g}));
    let Paint::Gradient(gp) = &doc(&s).swatch("Early").unwrap().paint else { panic!("a gradient") };
    assert_eq!((gp.gradient.kind, gp.gradient.stops.len(), gp.swatch.clone(), gp.geom), (vectorcraft_color::GradientKind::Radial, 2, None, None));
    assert_eq!(undo_len(&s), undo + 1);
    // Kinds don't mix; a colour replaces a colour.
    assert!(s.execute("swatch.edit", &json!({"name": "Early", "paint": {"color": "#00ff00"}})).is_err());
    run(&mut s, "swatch.edit", json!({"name": "Red", "paint": {"color": "#00ff00"}}));
    assert_eq!(doc(&s).swatch("Red").unwrap().paint.color().map(|c| c.to_hex()), Some("#00ff00".into()));
}
