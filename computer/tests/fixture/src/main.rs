//! A real accessible window for the semantic e2e. Controls: Salvar (counts
//! presses), Nome (text), Senha (password with a known value), Campo B
//! (password with a neutral name), Ativo (checkbox), Itens (list box
//! Um/Dois/Três), two OK buttons, a disabled button and Remover (removes
//! Salvar). Every accessibility action request and every physical click and
//! typed key is appended to $COMPUTER_A11Y_FIXTURE_LOG so the test can prove
//! exactly one effect per action and none per refusal.

use std::fs::OpenOptions;
use std::io::Write;

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, Toggled, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_winit::{Adapter, Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

const TITLE: &str = "computer a11y fixture";
pub const SECRET: &str = "s3cr3t-fixture";
pub const SECRET_B: &str = "campo-b-s3cr3t";

const WINDOW: NodeId = NodeId(0);
const SALVAR: NodeId = NodeId(1);
const NOME: NodeId = NodeId(2);
const SENHA: NodeId = NodeId(3);
const ATIVO: NodeId = NodeId(4);
const ITENS: NodeId = NodeId(5);
const UM: NodeId = NodeId(6);
const DOIS: NodeId = NodeId(7);
const TRES: NodeId = NodeId(8);
const OK1: NodeId = NodeId(9);
const OK2: NodeId = NodeId(10);
const DESABILITADO: NodeId = NodeId(11);
const REMOVER: NodeId = NodeId(12);
const STATUS: NodeId = NodeId(13);
const NOME_RUN: NodeId = NodeId(14);
const SENHA_RUN: NodeId = NodeId(15);
const CAMPO_B: NodeId = NodeId(16);
const CAMPO_B_RUN: NodeId = NodeId(17);

fn log(line: &str) {
    let Ok(path) = std::env::var("COMPUTER_A11Y_FIXTURE_LOG") else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

struct State {
    saved: u32,
    nome: String,
    ativo: bool,
    item: usize,
    salvar_present: bool,
}

fn name_of(id: NodeId) -> &'static str {
    match id {
        SALVAR => "Salvar",
        NOME => "Nome",
        SENHA => "Senha",
        CAMPO_B => "Campo B",
        ATIVO => "Ativo",
        UM => "Um",
        DOIS => "Dois",
        TRES => "Três",
        OK1 | OK2 => "OK",
        DESABILITADO => "Desabilitado",
        REMOVER => "Remover",
        _ => "other",
    }
}

fn button(label: &str) -> Node {
    let mut b = Node::new(Role::Button);
    b.set_label(label);
    b.add_action(Action::Click);
    b
}

/// The one text run a single-line input needs for AT-SPI to expose Text and
/// EditableText (AccessKit's `supports_text_ranges`). The platform trees
/// filter `TextRun` out, so no node is added on any OS.
fn run(text: &str) -> Node {
    let lengths: Vec<u8> = text.chars().map(|c| c.len_utf8() as u8).collect();
    let mut r = Node::new(Role::TextRun);
    r.set_value(text);
    r.set_character_lengths(lengths);
    r
}

fn password(label: &str, secret: &str, run_id: NodeId) -> Node {
    let mut p = Node::new(Role::PasswordInput);
    p.set_label(label);
    p.set_value(secret);
    p.set_children(vec![run_id]);
    p
}

fn build(s: &State) -> TreeUpdate {
    let mut children = Vec::new();
    let mut nodes = Vec::new();
    if s.salvar_present {
        children.push(SALVAR);
        nodes.push((SALVAR, button("Salvar")));
    }
    children.extend([
        NOME,
        SENHA,
        CAMPO_B,
        ATIVO,
        ITENS,
        OK1,
        OK2,
        DESABILITADO,
        REMOVER,
        STATUS,
    ]);

    let mut nome = Node::new(Role::TextInput);
    nome.set_label("Nome");
    nome.set_value(s.nome.as_str());
    nome.add_action(Action::SetValue);
    nome.set_children(vec![NOME_RUN]);
    nodes.push((NOME, nome));
    nodes.push((NOME_RUN, run(&s.nome)));

    nodes.push((SENHA, password("Senha", SECRET, SENHA_RUN)));
    nodes.push((SENHA_RUN, run(SECRET)));
    nodes.push((CAMPO_B, password("Campo B", SECRET_B, CAMPO_B_RUN)));
    nodes.push((CAMPO_B_RUN, run(SECRET_B)));

    let mut ativo = Node::new(Role::CheckBox);
    ativo.set_label("Ativo");
    ativo.set_toggled(if s.ativo {
        Toggled::True
    } else {
        Toggled::False
    });
    ativo.add_action(Action::Click);
    nodes.push((ATIVO, ativo));

    let mut itens = Node::new(Role::ListBox);
    itens.set_label("Itens");
    itens.set_children(vec![UM, DOIS, TRES]);
    nodes.push((ITENS, itens));
    for (i, id) in [UM, DOIS, TRES].into_iter().enumerate() {
        let mut item = Node::new(Role::ListBoxOption);
        item.set_label(name_of(id));
        item.add_action(Action::Click);
        item.set_selected(s.item == i);
        nodes.push((id, item));
    }

    nodes.push((OK1, button("OK")));
    nodes.push((OK2, button("OK")));
    let mut off = button("Desabilitado");
    off.set_disabled();
    nodes.push((DESABILITADO, off));
    nodes.push((REMOVER, button("Remover")));

    let status_text = format!("Status: salvo {}", s.saved);
    let mut status = Node::new(Role::Label);
    status.set_label(status_text.as_str());
    status.set_value(status_text.as_str());
    nodes.push((STATUS, status));

    let mut window = Node::new(Role::Window);
    window.set_label(TITLE);
    window.set_children(children);
    nodes.push((WINDOW, window));

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    }
}

/// Apply one request; `true` when the tree changed.
fn apply(req: &ActionRequest, s: &mut State) -> bool {
    match (req.target_node, req.action) {
        (SALVAR, Action::Click) if s.salvar_present => {
            s.saved += 1;
            true
        }
        (NOME, Action::SetValue) => match &req.data {
            Some(ActionData::Value(v)) => {
                s.nome = v.to_string();
                true
            }
            _ => false,
        },
        (ATIVO, Action::Click) => {
            s.ativo = !s.ativo;
            true
        }
        (UM, Action::Click) => {
            s.item = 0;
            true
        }
        (DOIS, Action::Click) => {
            s.item = 1;
            true
        }
        (TRES, Action::Click) => {
            s.item = 2;
            true
        }
        (REMOVER, Action::Click) => {
            s.salvar_present = false;
            true
        }
        _ => false,
    }
}

struct Main {
    window: Window,
    adapter: Adapter,
    state: State,
    cursor: (f64, f64),
}

struct App {
    proxy: EventLoopProxy<AccessKitEvent>,
    main: Option<Main>,
}

impl ApplicationHandler<AccessKitEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title(TITLE)
            .with_position(LogicalPosition::new(100.0, 100.0))
            .with_inner_size(LogicalSize::new(480.0, 640.0))
            .with_visible(false);
        let window = event_loop.create_window(attrs).expect("create window");
        let mut adapter = Adapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());
        window.set_visible(true);
        adapter.process_event(&window, &WindowEvent::Focused(true));
        let pos = window.inner_position().unwrap_or_default();
        let size = window.inner_size();
        log(&format!(
            "window {} {} {} {} {}",
            pos.x,
            pos.y,
            size.width,
            size.height,
            window.scale_factor()
        ));
        self.main = Some(Main {
            window,
            adapter,
            state: State {
                saved: 0,
                nome: String::new(),
                ativo: false,
                item: 0,
                salvar_present: true,
            },
            cursor: (0.0, 0.0),
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(main) = self.main.as_mut() else {
            return;
        };
        main.adapter.process_event(&main.window, &event);
        match event {
            WindowEvent::CursorMoved { position, .. } => main.cursor = (position.x, position.y),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => {
                log(&format!(
                    "mouse {} {}",
                    main.cursor.0.round(),
                    main.cursor.1.round()
                ));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(text) = &event.text {
                    log(&format!("key {}", text.as_str().escape_debug()));
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AccessKitEvent) {
        let Some(main) = self.main.as_mut() else {
            return;
        };
        match event.window_event {
            AccessKitWindowEvent::InitialTreeRequested => {
                main.adapter.update_if_active(|| build(&main.state));
            }
            AccessKitWindowEvent::ActionRequested(req) => {
                log(&format!(
                    "action {} {:?} {:?}",
                    name_of(req.target_node),
                    req.action,
                    req.data
                ));
                if apply(&req, &mut main.state) {
                    main.adapter.update_if_active(|| build(&main.state));
                }
            }
            AccessKitWindowEvent::AccessibilityDeactivated => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App {
        proxy: event_loop.create_proxy(),
        main: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
