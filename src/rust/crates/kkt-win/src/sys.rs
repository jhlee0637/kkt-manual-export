//! Win32 호출. Windows 가 아니면 모든 함수가 "없음" 값을 돌려주고 `SUPPORTED` 가 false 다.
//! 안전 규칙: 키는 대상 창이 포커스일 때만 보낸다 (호출하는 쪽에서 확인).

pub type Hwnd = isize;

pub const VK_CONTROL: u16 = 0x11;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_MENU: u16 = 0x12;
pub const VK_S: u16 = 0x53;
pub const VK_D: u16 = 0x44;

pub const SW_MAXIMIZE: i32 = 3;
pub const SW_MINIMIZE: i32 = 6;
pub const SW_RESTORE: i32 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 캡처한 창 이미지. 픽셀은 위에서 아래로, 한 픽셀이 B,G,R,A 순서의 4바이트.
#[derive(Debug, Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Image {
    pub fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.w + x) * 4;
        (self.px[i + 2], self.px[i + 1], self.px[i])
    }
}

pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_ESCAPE: u16 = 0x1B;

#[cfg(windows)]
pub use imp::*;
#[cfg(not(windows))]
pub use stub::*;

#[cfg(windows)]
mod imp {
    use super::*;

    pub const SUPPORTED: bool = true;

    const WM_SETTEXT: u32 = 0x000C;
    const WM_GETTEXT: u32 = 0x000D;
    const WM_GETTEXTLENGTH: u32 = 0x000E;
    const WM_CLOSE: u32 = 0x0010;
    const BM_CLICK: u32 = 0x00F5;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_KEYUP: u32 = 2;

    type EnumProc = unsafe extern "system" fn(Hwnd, isize) -> i32;

    #[repr(C)]
    struct Rect32 {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        rc_monitor: Rect32,
        rc_work: Rect32,
        dw_flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeybdInput {
        w_vk: u16,
        w_scan: u16,
        dw_flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MouseInput {
        dx: i32,
        dy: i32,
        mouse_data: u32,
        dw_flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    #[repr(C)]
    union InputUnion {
        ki: KeybdInput,
        mi: MouseInput,
    }

    #[repr(C)]
    struct Input {
        kind: u32,
        u: InputUnion,
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(f: EnumProc, l: isize) -> i32;
        fn EnumChildWindows(h: Hwnd, f: EnumProc, l: isize) -> i32;
        fn IsWindow(h: Hwnd) -> i32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn IsIconic(h: Hwnd) -> i32;
        fn IsZoomed(h: Hwnd) -> i32;
        fn GetClassNameW(h: Hwnd, buf: *mut u16, max: i32) -> i32;
        fn GetWindowTextLengthW(h: Hwnd) -> i32;
        fn GetWindowTextW(h: Hwnd, buf: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
        fn GetParent(h: Hwnd) -> Hwnd;
        fn GetDlgCtrlID(h: Hwnd) -> i32;
        fn SendMessageW(h: Hwnd, msg: u32, w: usize, l: isize) -> isize;
        fn PostMessageW(h: Hwnd, msg: u32, w: usize, l: isize) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn SetForegroundWindow(h: Hwnd) -> i32;
        fn GetWindowRect(h: Hwnd, r: *mut Rect32) -> i32;
        fn MonitorFromWindow(h: Hwnd, flags: u32) -> isize;
        fn GetMonitorInfoW(m: isize, mi: *mut MonitorInfo) -> i32;
        fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
        fn SetWindowPos(h: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        fn SendInput(n: u32, inputs: *const Input, size: i32) -> u32;
        fn GetAsyncKeyState(vk: i32) -> i16;
        fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
        fn GetDC(h: Hwnd) -> isize;
        fn ReleaseDC(h: Hwnd, dc: isize) -> i32;
        fn PrintWindow(h: Hwnd, dc: isize, flags: u32) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn GetCursorPos(p: *mut Point) -> i32;
        fn mouse_event(flags: u32, dx: u32, dy: u32, data: u32, extra: usize);
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_ppm: i32,
        y_ppm: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 1],
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateCompatibleDC(dc: isize) -> isize;
        fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
        fn SelectObject(dc: isize, obj: isize) -> isize;
        fn DeleteObject(obj: isize) -> i32;
        fn DeleteDC(dc: isize) -> i32;
        fn GetDIBits(dc: isize, bmp: isize, start: u32, lines: u32, bits: *mut u8, bi: *mut BitmapInfo, usage: u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleOutputCP(cp: u32) -> i32;
    }

    #[link(name = "shcore")]
    extern "system" {
        fn SetProcessDpiAwareness(level: i32) -> i32;
    }

    pub fn ctrl_d_pressed() -> bool {
        unsafe { (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0 && (GetAsyncKeyState(VK_D as i32) as u16 & 0x8000) != 0 }
    }

    /// 좌표가 물리 픽셀 기준이 되도록. 배율이 100%가 아닐 때 클릭이 빗나가는 것을 막는다.
    pub fn set_dpi_aware() {
        unsafe {
            if SetProcessDpiAwarenessContext(-4) != 0 {
                return;
            }
            SetProcessDpiAwareness(2);
        }
    }

    pub fn pid_of(h: Hwnd) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, &mut pid) };
        pid
    }

    pub fn class_of(h: Hwnd) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), 256) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    pub fn text_of(h: Hwnd) -> String {
        unsafe {
            let n = SendMessageW(h, WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
            let mut buf = vec![0u16; n + 1];
            let got = SendMessageW(h, WM_GETTEXT, n + 1, buf.as_mut_ptr() as isize).max(0) as usize;
            String::from_utf16_lossy(&buf[..got.min(n)])
        }
    }

    fn window_title(h: Hwnd) -> String {
        unsafe {
            let n = GetWindowTextLengthW(h).max(0) as usize;
            let mut buf = vec![0u16; n + 1];
            let got = GetWindowTextW(h, buf.as_mut_ptr(), (n + 1) as i32).max(0) as usize;
            String::from_utf16_lossy(&buf[..got.min(n)])
        }
    }

    pub fn set_text(h: Hwnd, value: &str) {
        let mut buf: Vec<u16> = value.encode_utf16().collect();
        buf.push(0);
        unsafe { SendMessageW(h, WM_SETTEXT, 0, buf.as_ptr() as isize) };
    }

    pub fn click_button(h: Hwnd) {
        unsafe { SendMessageW(h, BM_CLICK, 0, 0) };
    }

    pub fn close_window(h: Hwnd) {
        unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
    }

    pub fn is_window(h: Hwnd) -> bool {
        unsafe { IsWindow(h) != 0 }
    }

    pub fn is_visible(h: Hwnd) -> bool {
        unsafe { IsWindow(h) != 0 && IsWindowVisible(h) != 0 }
    }

    pub fn foreground() -> Hwnd {
        unsafe { GetForegroundWindow() }
    }

    pub fn set_foreground(h: Hwnd) {
        unsafe { SetForegroundWindow(h) };
    }

    unsafe extern "system" fn collect_cb(h: Hwnd, l: isize) -> i32 {
        // l 은 호출한 쪽이 넘긴 `(&mut Vec<Hwnd>)` 이다.
        let v = &mut *(l as *mut Vec<Hwnd>);
        v.push(h);
        1
    }

    fn all_top_level() -> Vec<Hwnd> {
        let mut v: Vec<Hwnd> = Vec::new();
        unsafe { EnumWindows(collect_cb, &mut v as *mut Vec<Hwnd> as isize) };
        v
    }

    /// 제목이 정확히 일치하는 보이는 최상위 창 중 클래스가 `class` 인 것.
    pub fn find_top_level(class: &str, title: &str) -> Vec<Hwnd> {
        all_top_level()
            .into_iter()
            .filter(|&h| unsafe { IsWindowVisible(h) } != 0 && class_of(h) == class && window_title(h) == title)
            .collect()
    }

    /// 클래스가 `class` 인 보이는 최상위 창 `(hwnd, title)`.
    pub fn list_top_level(class: &str) -> Vec<(Hwnd, String)> {
        all_top_level()
            .into_iter()
            .filter(|&h| unsafe { IsWindowVisible(h) } != 0 && class_of(h) == class)
            .map(|h| (h, window_title(h)))
            .collect()
    }

    /// 콘솔 출력을 UTF-8 로 맞춘다 (더블클릭으로 연 콘솔의 한글이 깨지지 않게).
    pub fn set_console_utf8() {
        unsafe { SetConsoleOutputCP(65001) };
    }

    /// 콘솔 창의 보이는 폭(칸). 콘솔이 아니면(파이프 등) `None`.
    pub fn console_width() -> Option<usize> {
        #[repr(C)]
        struct Coord {
            x: i16,
            y: i16,
        }
        #[repr(C)]
        struct SmallRect {
            left: i16,
            top: i16,
            right: i16,
            bottom: i16,
        }
        #[repr(C)]
        struct ScreenInfo {
            size: Coord,
            cursor: Coord,
            attributes: u16,
            window: SmallRect,
            max_size: Coord,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(which: u32) -> isize;
            fn GetConsoleScreenBufferInfo(h: isize, info: *mut ScreenInfo) -> i32;
        }
        unsafe {
            let h = GetStdHandle(0xFFFF_FFF5); // STD_OUTPUT_HANDLE (-11)
            let mut info = std::mem::zeroed::<ScreenInfo>();
            if h == 0 || h == -1 || GetConsoleScreenBufferInfo(h, &mut info) == 0 {
                return None;
            }
            let w = info.window.right as i32 - info.window.left as i32 + 1;
            (w > 0).then_some(w as usize)
        }
    }

    /// 해당 프로세스의 보이는 최상위 창 `(hwnd, class, title)`. 제목 없는 완료 팝업도 포함한다.
    pub fn top_level_windows(pid: u32) -> Vec<(Hwnd, String, String)> {
        all_top_level()
            .into_iter()
            .filter(|&h| unsafe { IsWindowVisible(h) } != 0 && pid_of(h) == pid)
            .map(|h| (h, class_of(h), text_of(h)))
            .collect()
    }

    /// 해당 프로세스의 보이는 `#32770`(공용 대화상자) 최상위 창.
    pub fn top_level_dialogs(pid: u32) -> Vec<Hwnd> {
        top_level_windows(pid).into_iter().filter(|(_, c, _)| c == "#32770").map(|(h, _, _)| h).collect()
    }

    /// 모든 하위 창 `(hwnd, class, parent_class, ctrl_id)`.
    pub fn descendants(h: Hwnd) -> Vec<(Hwnd, String, String, i32)> {
        let mut v: Vec<Hwnd> = Vec::new();
        unsafe { EnumChildWindows(h, collect_cb, &mut v as *mut Vec<Hwnd> as isize) };
        v.into_iter()
            .map(|c| {
                let p = unsafe { GetParent(c) };
                (c, class_of(c), if p != 0 { class_of(p) } else { String::new() }, unsafe { GetDlgCtrlID(c) })
            })
            .collect()
    }

    pub fn window_rect(h: Hwnd) -> Option<RawRect> {
        let mut r = Rect32 { left: 0, top: 0, right: 0, bottom: 0 };
        (unsafe { GetWindowRect(h, &mut r) } != 0).then_some(RawRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
    }

    /// 창이 놓인 모니터의 작업 영역(작업표시줄 제외).
    pub fn work_area(h: Hwnd) -> Option<RawRect> {
        unsafe {
            let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
            let mut mi = MonitorInfo {
                cb_size: std::mem::size_of::<MonitorInfo>() as u32,
                rc_monitor: Rect32 { left: 0, top: 0, right: 0, bottom: 0 },
                rc_work: Rect32 { left: 0, top: 0, right: 0, bottom: 0 },
                dw_flags: 0,
            };
            if GetMonitorInfoW(mon, &mut mi) == 0 {
                return None;
            }
            let w = mi.rc_work;
            Some(RawRect { left: w.left, top: w.top, right: w.right, bottom: w.bottom })
        }
    }

    /// 창을 (가려져 있어도) 그대로 캡처한다. 포커스를 가져가지 않는다.
    pub fn capture_window(h: Hwnd) -> Option<Image> {
        let r = window_rect(h)?;
        let (w, hgt) = ((r.right - r.left).max(0), (r.bottom - r.top).max(0));
        if w == 0 || hgt == 0 {
            return None;
        }
        unsafe {
            let screen = GetDC(0);
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, hgt);
            let old = SelectObject(mem, bmp);
            let ok = PrintWindow(h, mem, 2) != 0; // 2 = PW_RENDERFULLCONTENT
            let mut px = vec![0u8; (w * hgt * 4) as usize];
            let mut bi = BitmapInfo {
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width: w,
                    height: -hgt, // 위에서 아래로
                    planes: 1,
                    bit_count: 32,
                    compression: 0,
                    size_image: 0,
                    x_ppm: 0,
                    y_ppm: 0,
                    clr_used: 0,
                    clr_important: 0,
                },
                colors: [0],
            };
            let got = GetDIBits(mem, bmp, 0, hgt as u32, px.as_mut_ptr(), &mut bi, 0);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(0, screen);
            (ok && got != 0).then_some(Image { w: w as usize, h: hgt as usize, px })
        }
    }

    pub fn cursor_to(x: i32, y: i32) {
        unsafe { SetCursorPos(x, y) };
    }

    /// 지금 마우스 커서의 화면 좌표.
    pub fn cursor_pos() -> (i32, i32) {
        let mut p = Point { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut p) };
        (p.x, p.y)
    }

    pub fn mouse_click() {
        unsafe {
            mouse_event(0x0002, 0, 0, 0, 0); // LEFTDOWN
            std::thread::sleep(std::time::Duration::from_millis(50));
            mouse_event(0x0004, 0, 0, 0, 0); // LEFTUP
        }
    }

    /// 휠을 `notches` 칸 돌린다 (음수는 아래로).
    pub fn wheel(notches: i32) {
        unsafe { mouse_event(0x0800, 0, 0, (notches * 120) as u32, 0) };
    }

    pub fn is_iconic(h: Hwnd) -> bool {
        unsafe { IsIconic(h) != 0 }
    }

    pub fn is_zoomed(h: Hwnd) -> bool {
        unsafe { IsZoomed(h) != 0 }
    }

    pub fn show_window(h: Hwnd, cmd: i32) {
        unsafe { ShowWindow(h, cmd) };
    }

    /// 포커스를 가져가지 않고 크기와 위치만 바꾼다.
    pub fn set_window_rect(h: Hwnd, left: i32, top: i32, width: i32, height: i32) -> bool {
        unsafe { SetWindowPos(h, 0, left, top, width, height, SWP_NOZORDER | SWP_NOACTIVATE) != 0 }
    }

    /// `events`: `(vk, down)`. 유니코드 문자는 쓰지 않는다.
    pub fn send_keys(events: &[(u16, bool)]) -> Result<(), String> {
        let inputs: Vec<Input> = events
            .iter()
            .map(|&(vk, down)| Input {
                kind: INPUT_KEYBOARD,
                u: InputUnion { ki: KeybdInput { w_vk: vk, w_scan: 0, dw_flags: if down { 0 } else { KEYEVENTF_KEYUP }, time: 0, dw_extra_info: 0 } },
            })
            .collect();
        let n = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<Input>() as i32) } as usize;
        if n != events.len() {
            return Err(format!("SendInput 이 {n}/{}개만 주입했다 (권한이 높은 창이 포커스일 수 있다)", events.len()));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod stub {
    use super::*;

    pub const SUPPORTED: bool = false;

    pub fn ctrl_d_pressed() -> bool { false }
    pub fn set_dpi_aware() {}
    pub fn pid_of(_: Hwnd) -> u32 { 0 }
    pub fn class_of(_: Hwnd) -> String { String::new() }
    pub fn text_of(_: Hwnd) -> String { String::new() }
    pub fn set_text(_: Hwnd, _: &str) {}
    pub fn click_button(_: Hwnd) {}
    pub fn close_window(_: Hwnd) {}
    pub fn is_window(_: Hwnd) -> bool { false }
    pub fn is_visible(_: Hwnd) -> bool { false }
    pub fn foreground() -> Hwnd { 0 }
    pub fn set_foreground(_: Hwnd) {}
    pub fn find_top_level(_: &str, _: &str) -> Vec<Hwnd> { Vec::new() }
    pub fn list_top_level(_: &str) -> Vec<(Hwnd, String)> { Vec::new() }
    pub fn set_console_utf8() {}
    pub fn console_width() -> Option<usize> { None }
    pub fn top_level_windows(_: u32) -> Vec<(Hwnd, String, String)> { Vec::new() }
    pub fn top_level_dialogs(_: u32) -> Vec<Hwnd> { Vec::new() }
    pub fn descendants(_: Hwnd) -> Vec<(Hwnd, String, String, i32)> { Vec::new() }
    pub fn window_rect(_: Hwnd) -> Option<RawRect> { None }
    pub fn work_area(_: Hwnd) -> Option<RawRect> { None }
    pub fn capture_window(_: Hwnd) -> Option<Image> { None }
    pub fn cursor_to(_: i32, _: i32) {}
    pub fn cursor_pos() -> (i32, i32) { (0, 0) }
    pub fn mouse_click() {}
    pub fn wheel(_: i32) {}
    pub fn is_iconic(_: Hwnd) -> bool { false }
    pub fn is_zoomed(_: Hwnd) -> bool { false }
    pub fn show_window(_: Hwnd, _: i32) {}
    pub fn set_window_rect(_: Hwnd, _: i32, _: i32, _: i32, _: i32) -> bool { false }
    pub fn send_keys(_: &[(u16, bool)]) -> Result<(), String> { Err("Windows 에서만 동작한다".into()) }
}

/// 키 하나를 보낸다. 수정키를 확실히 떼도록 눌렀던 것은 반드시 뗀다.
pub fn tap(vk: u16, guard: &crate::guard::Guard, mods: &[u16]) -> Result<(), crate::WinError> {
    guard.check()?;
    let mut downs: Vec<(u16, bool)> = mods.iter().map(|&m| (m, true)).collect();
    downs.push((vk, true));
    let mut ups: Vec<(u16, bool)> = vec![(vk, false)];
    ups.extend(mods.iter().rev().map(|&m| (m, false)));
    let sent = send_keys(&downs);
    if sent.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    let _ = send_keys(&ups);
    sent.map_err(crate::WinError::Collect)
}

/// 대상 창을 앞으로 가져오고 실제로 포커스가 갔는지 확인한다. 실패하면 `Ok(false)`.
pub fn bring_to_front(h: Hwnd, guard: &crate::guard::Guard, timeout: f64) -> Result<bool, crate::WinError> {
    if foreground() == h {
        return Ok(true);
    }
    let _ = send_keys(&[(VK_MENU, true), (VK_MENU, false)]); // 포그라운드 전환 제한을 푸는 관용 기법
    set_foreground(h);
    Ok(guard.wait_until(|| (foreground() == h).then_some(()), timeout, 0.05)?.is_some())
}
