//! Keeps the window buttons at the configured `trafficLightPosition`.
//!
//! AppKit puts them back in their default spot whenever it lays out the title bar again: when the
//! appearance changes, when the window becomes key, or when an accessibility client such as VoiceOver
//! attaches. tao only restores the configured spot on its own redraw, which may not come until the window
//! is resized. With an accessibility client attached, AppKit also aligns the buttons to the top of the
//! title bar instead of keeping their usual offset from its bottom.
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{NSButton, NSView, NSViewFrameDidChangeNotification, NSWindow, NSWindowButton};
use objc2_foundation::{NSNotificationCenter, NSOperationQueue};

/// The title bar container holding the buttons, two levels up as tao finds it.
fn title_bar(close: &NSButton) -> Option<Retained<NSView>> {
    // The views are only read on the main thread, where setup and the frame-change handler run.
    unsafe { close.superview().and_then(|v| v.superview()) }
}

/// AppKit's usual layout of the buttons: their offset from the bottom of the title bar and the distance
/// between neighbours.
#[derive(Clone, Copy)]
struct Usual {
    bottom: f64,
    spacing: f64,
}

/// Same arithmetic as tao's own inset, so both agree on where the buttons belong.
fn place(window: &NSWindow, x: f64, y: f64, usual: Usual) {
    let buttons = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .map(|b| window.standardWindowButton(b));
    let [Some(close), Some(miniaturize), Some(zoom)] = buttons else {
        return;
    };
    let Some(bar) = title_bar(&close) else {
        return;
    };
    let height = close.frame().size.height + y;
    let mut frame = bar.frame();
    let top = window.frame().size.height - height;
    // Moving a view posts another frame change; leaving views already in place ends that loop.
    if frame.size.height != height || frame.origin.y != top {
        frame.size.height = height;
        frame.origin.y = top;
        bar.setFrame(frame);
    }
    for (i, button) in [close, miniaturize, zoom].iter().enumerate() {
        let mut origin = button.frame().origin;
        let target = x + i as f64 * usual.spacing;
        if origin.x != target || origin.y != usual.bottom {
            origin.x = target;
            origin.y = usual.bottom;
            button.setFrameOrigin(origin);
        }
    }
}

/// Must run on the main thread with the window's NSWindow.
pub fn pin(window: Retained<NSWindow>, x: f64, y: f64) {
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(bar) = title_bar(&close) else {
        return;
    };
    let Some(miniaturize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    // setup runs before the main run loop answers any accessibility client, so this is the usual layout.
    let usual = Usual {
        bottom: close.frame().origin.y,
        spacing: miniaturize.frame().origin.x - close.frame().origin.x,
    };
    let watched: [Retained<NSView>; 2] = [Retained::into_super(Retained::into_super(close)), bar];
    let handler = {
        let window = window.clone();
        RcBlock::new(move |_| place(&window, x, y, usual))
    };
    let center = NSNotificationCenter::defaultCenter();
    // A frame change arrives in the middle of AppKit's own layout, which overwrites anything set then;
    // the main queue runs the handler once that layout is done.
    let main = NSOperationQueue::mainQueue();
    for view in &watched {
        view.setPostsFrameChangedNotifications(true);
        // The center keeps the observer for the window's lifetime, which is the app's.
        unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSViewFrameDidChangeNotification),
                Some(view),
                Some(&main),
                &handler,
            );
        }
    }
    place(&window, x, y, usual);
}
