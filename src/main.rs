use rand::Rng;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use image::RgbaImage;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder,
};
use winit::{
    dpi::PhysicalPosition,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowId},
};
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::UI::WindowsAndMessaging::*,
};

// 图片缓存结构
struct CachedImage {
    name: String,
    data: RgbaImage,
}

// 全局状态：存储按键序列
static mut KEY_SEQUENCE: Vec<char> = Vec::new();
static mut EVENT_LOOP_PROXY: Option<winit::event_loop::EventLoopProxy<CustomEvent>> = None;
static IMAGE_CACHE: Mutex<Vec<CachedImage>> = Mutex::new(Vec::new());

#[derive(Debug, Clone)]
enum CustomEvent {
    ShowImage,
    Exit,
}

// 键盘钩子回调函数
unsafe extern "system" fn keyboard_hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 && w_param.0 == WM_KEYDOWN as usize {
        let kb_struct = *(l_param.0 as *const KBDLLHOOKSTRUCT);
        let vk_code = kb_struct.vkCode;

        // 将虚拟键码转换为字符
        let key_char = match vk_code {
            0x48 => Some('h'), // H key
            0x59 => Some('y'), // Y key
            0x57 => Some('w'), // W key
            _ => None,
        };

        if let Some(c) = key_char {
            KEY_SEQUENCE.push(c);

            // 只保留最后3个按键
            if KEY_SEQUENCE.len() > 3 {
                KEY_SEQUENCE.remove(0);
            }

            // 检查是否按下了 "hyw" 序列
            if KEY_SEQUENCE.len() == 3 {
                let sequence: String = KEY_SEQUENCE.iter().collect();
                if sequence == "hyw" {
                    println!("Detected 'hyw' sequence!");
                    KEY_SEQUENCE.clear();

                    // 触发显示图片事件
                    if let Some(proxy) = &EVENT_LOOP_PROXY {
                        let _ = proxy.send_event(CustomEvent::ShowImage);
                    }
                }
            }
        }
    }

    CallNextHookEx(HHOOK::default(), n_code, w_param, l_param)
}

// 在启动时加载所有图片到内存
fn load_images_to_cache() {
    let img_dir = PathBuf::from("img");

    if !img_dir.exists() {
        eprintln!("img folder not found!");
        return;
    }

    let entries: Vec<PathBuf> = fs::read_dir(&img_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    matches!(
                        ext.to_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp"
                    )
                })
                .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        eprintln!("No images found in img folder!");
        return;
    }

    println!("Loading {} images into cache...", entries.len());

    let mut cache = IMAGE_CACHE.lock().unwrap();

    for path in entries {
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        match image::open(&path) {
            Ok(img) => {
                let rgba_img = img.to_rgba8();
                cache.push(CachedImage {
                    name: name.clone(),
                    data: rgba_img,
                });
                println!("  Loaded: {}", name);
            }
            Err(e) => {
                eprintln!("  Failed to load {}: {}", name, e);
            }
        }
    }

    println!("Image cache ready with {} images", cache.len());
}

// 从缓存中获取随机图片
fn get_random_cached_image() -> Option<(String, RgbaImage)> {
    let cache = IMAGE_CACHE.lock().unwrap();

    if cache.is_empty() {
        eprintln!("Image cache is empty!");
        return None;
    }

    let mut rng = rand::thread_rng();
    let index = rng.gen_range(0..cache.len());
    let cached = &cache[index];

    Some((cached.name.clone(), cached.data.clone()))
}

struct ImageWindow {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    created_at: Instant,
}

fn main() -> Result<()> {
    println!("Starting HYW Image Viewer...");

    // 启动时加载所有图片到内存缓存
    load_images_to_cache();

    // 创建事件循环
    let event_loop = EventLoop::<CustomEvent>::with_user_event().build().unwrap();

    // 保存事件循环代理
    unsafe {
        EVENT_LOOP_PROXY = Some(event_loop.create_proxy());
    }

    // 创建系统托盘
    let tray_menu = Menu::new();
    let quit_item = MenuItem::new("退出", true, None);
    tray_menu.append(&quit_item).unwrap();

    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("HYW Image Viewer")
        .build()
        .unwrap();

    // 设置全局键盘钩子
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null())
            .ok()
            .map(|h| HINSTANCE(h.0))
            .unwrap_or_default();

        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            hinstance,
            0,
        )?;

        if hook.is_invalid() {
            return Err(Error::from_win32());
        }
    }

    // 监听托盘菜单事件
    let menu_channel = MenuEvent::receiver();
    let event_loop_proxy = event_loop.create_proxy();
    let quit_item_id = quit_item.id().clone();

    std::thread::spawn(move || loop {
        if let Ok(event) = menu_channel.recv() {
            if event.id == quit_item_id {
                let _ = event_loop_proxy.send_event(CustomEvent::Exit);
                break;
            }
        }
    });

    // 运行事件循环
    let mut image_windows: HashMap<WindowId, ImageWindow> = HashMap::new();

    event_loop
        .run(move |event, event_loop_target| {
            event_loop_target.set_control_flow(ControlFlow::Wait);

            match event {
                Event::UserEvent(CustomEvent::ShowImage) => {
                    // 从缓存中获取随机图片
                    if let Some((image_name, img)) = get_random_cached_image() {
                        println!("Showing image: {}", image_name);

                        let (img_width, img_height) = img.dimensions();

                        // 获取屏幕尺寸
                        if let Some(monitor) = event_loop_target.primary_monitor() {
                            let screen_size = monitor.size();
                            let screen_height = screen_size.height as f32;

                            // 计算窗口尺寸：高度为屏幕高度的一半，宽度按比例
                            let window_height = screen_height / 2.0;
                            let aspect_ratio = img_width as f32 / img_height as f32;
                            let window_width = window_height * aspect_ratio;

                            // 创建窗口
                            let window_attrs = Window::default_attributes()
                                .with_title("Image Viewer")
                                .with_inner_size(winit::dpi::PhysicalSize::new(
                                    window_width as u32,
                                    window_height as u32,
                                ))
                                .with_decorations(false)
                                .with_transparent(false)
                                .with_visible(true)
                                .with_window_level(winit::window::WindowLevel::AlwaysOnTop);

                            if let Ok(window) = event_loop_target.create_window(window_attrs) {
                                let window: Rc<Window> = Rc::new(window);

                                // 居中显示
                                let monitor_pos = monitor.position();
                                let monitor_size = monitor.size();
                                let x = monitor_pos.x
                                    + (monitor_size.width as i32 - window_width as i32) / 2;
                                let y = monitor_pos.y
                                    + (monitor_size.height as i32 - window_height as i32) / 2;
                                window.set_outer_position(PhysicalPosition::new(x, y));

                                // 创建软件渲染表面
                                let context = softbuffer::Context::new(window.clone()).unwrap();
                                let mut surface =
                                    softbuffer::Surface::new(&context, window.clone()).unwrap();

                                // 调整图片大小以适应窗口
                                let resized_img = image::imageops::resize(
                                    &img,
                                    window_width as u32,
                                    window_height as u32,
                                    image::imageops::FilterType::Lanczos3,
                                );

                                // 渲染图片
                                surface
                                    .resize(
                                        NonZeroU32::new(window_width as u32).unwrap(),
                                        NonZeroU32::new(window_height as u32).unwrap(),
                                    )
                                    .unwrap();

                                let mut buffer = surface.buffer_mut().unwrap();
                                for (i, pixel) in resized_img.pixels().enumerate() {
                                    let r = pixel[0] as u32;
                                    let g = pixel[1] as u32;
                                    let b = pixel[2] as u32;
                                    buffer[i] = (r << 16) | (g << 8) | b;
                                }
                                buffer.present().unwrap();

                                // 保存窗口信息
                                let window_id = window.id();
                                image_windows.insert(
                                    window_id,
                                    ImageWindow {
                                        window,
                                        surface,
                                        created_at: Instant::now(),
                                    },
                                );

                                // 设置定时器检查
                                event_loop_target.set_control_flow(ControlFlow::Poll);
                            }
                        }
                    }
                }
                Event::UserEvent(CustomEvent::Exit) => {
                    println!("Exiting...");
                    event_loop_target.exit();
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                } => {
                    // 移除关闭的窗口
                    image_windows.remove(&window_id);
                }
                Event::AboutToWait => {
                    // 检查是否有窗口需要关闭（显示超过1秒）
                    let now = Instant::now();
                    let windows_to_close: Vec<WindowId> = image_windows
                        .iter()
                        .filter(|(_, win)| now.duration_since(win.created_at) > Duration::from_secs(1))
                        .map(|(id, _)| *id)
                        .collect();

                    for window_id in windows_to_close {
                        image_windows.remove(&window_id);
                    }

                    // 如果没有图片窗口，切换到等待模式
                    if image_windows.is_empty() {
                        event_loop_target.set_control_flow(ControlFlow::Wait);
                    }
                }
                _ => {}
            }
        })
        .unwrap();

    Ok(())
}
