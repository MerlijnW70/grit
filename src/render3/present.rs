//! The picture shown in a window straight from the device.
//!
//! [`window`] stretches the picture over the window's pixels and lays a
//! picture drawn by the host over it, both in `PIXELS`, after the words of
//! [`Layout::words`]. A [`Presenter`] copies those words into an image of a
//! Vulkan swapchain on the window and presents it, on the queue the
//! picture's passes run on, so nothing is read back to the host.
//!
//! The overlay holds a word a window pixel: its colour already weighed by
//! how much it covers, and how much it covers, out of 255, in the top byte.
//! A word of 0 lets the picture through unchanged.

use ash::vk;
use simdr::lanes::LaneError;
use simdr::module::Id;

use super::emit::Em;
use super::shade::{kernel, placed};
use super::{Build, Layout, Screen, refuse, word};
use crate::Host;
use crate::layout::PIXELS;
use crate::resident::Resident;

/// Where the window's words lie in `PIXELS`: its colours, then its overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windowed {
    pub window: Screen,
    pub colours: usize,
    pub overlay: usize,
}

impl Windowed {
    #[must_use]
    pub const fn of(layout: &Layout, window: Screen) -> Self {
        let colours = layout.words();
        Self {
            window,
            colours,
            overlay: colours + window.pixels(),
        }
    }

    /// The words `PIXELS` holds for the picture and the window.
    #[must_use]
    pub const fn words(&self) -> usize {
        self.overlay + self.window.pixels()
    }
}

/// Where pixel `at` of a line `to` long reads a line `from` long: the first
/// pixel, the next, and the share of the next, out of 256.
#[must_use]
pub fn tap(at: u32, from: u32, to: u32) -> (u32, u32, u32) {
    let to = to.max(1);
    let last = from.saturating_sub(1);
    let across = (2 * at + 1) * from;
    let gone = across.saturating_sub(to);
    let place = (gone / to) * 128 + ((gone % to) * 128) / to;
    let low = (place >> 8).min(last);
    let high = (low + 1).min(last);
    let share = if place >> 8 > last { 0 } else { place & 0xFF };
    (low, high, share)
}

/// Two pixels, `0x00RRGGBB`, blended by `share` out of 256 toward `b`.
#[must_use]
pub const fn blend(a: u32, b: u32, share: u32) -> u32 {
    let keep = 256 - share;
    let red_blue = ((a & 0x00FF_00FF) * keep + (b & 0x00FF_00FF) * share) >> 8;
    let green = ((a & 0x0000_FF00) * keep + (b & 0x0000_FF00) * share) >> 8;
    (red_blue & 0x00FF_00FF) | (green & 0x0000_FF00)
}

/// `pixel` under the overlay word `over`.
#[must_use]
pub const fn covered(pixel: u32, over: u32) -> u32 {
    let cover = over >> 24;
    let keep = 255 - cover;
    let mut out = 0;
    let mut shift = 0;
    while shift < 24 {
        let shown = (pixel >> shift) & 0xFF;
        let laid = (over >> shift) & 0xFF;
        let value = laid + (shown * keep + 127) / 255;
        let value = if value > 255 { 255 } else { value };
        out |= value << shift;
        shift += 8;
    }
    out
}

/// `picture`, drawn on `from`, stretched over `to`, blended between the four
/// pixels around each, under `overlay`, as [`window`] writes it.
#[must_use]
pub fn windowed(picture: &[u32], from: Screen, to: Screen, overlay: &[u32]) -> Vec<u32> {
    let across: Vec<_> = (0..to.wide).map(|x| tap(x, from.wide, to.wide)).collect();
    let wide = from.wide as usize;
    let get = |x: u32, y: u32| {
        picture
            .get(y as usize * wide + x as usize)
            .copied()
            .unwrap_or(0)
    };
    let mut out = Vec::with_capacity(to.pixels());
    for y in 0..to.high {
        let (top, bottom, share_y) = tap(y, from.high, to.high);
        for &(left, right, share_x) in &across {
            let upper = blend(get(left, top), get(right, top), share_x);
            let lower = blend(get(left, bottom), get(right, bottom), share_x);
            let pixel = blend(upper, lower, share_y);
            let over = overlay.get(out.len()).copied().unwrap_or(0);
            out.push(covered(pixel, over));
        }
    }
    out
}

fn tap_em(em: &mut Em<'_>, at: Id, from: u32, to: u32) -> Result<(Id, Id, Id), LaneError> {
    let to = to.max(1);
    let last = em.u(from.saturating_sub(1))?;
    let twice = em.add(at, at)?;
    let one = em.u(1)?;
    let odd = em.add(twice, one)?;
    let from_word = em.u(from)?;
    let across = em.mul(odd, from_word)?;
    let to_word = em.u(to)?;
    let short = em.lt(across, to_word)?;
    let gone = em.sub(across, to_word)?;
    let none = em.u(0)?;
    let gone = em.pick(short, none, gone)?;
    let quotient = em.div(gone, to_word)?;
    let whole = em.shl(quotient, 7)?;
    let back = em.mul(quotient, to_word)?;
    let rest = em.sub(gone, back)?;
    let rest = em.shl(rest, 7)?;
    let rest = em.div(rest, to_word)?;
    let place = em.add(whole, rest)?;
    let low = em.shr(place, 8)?;
    let past = em.lt(last, low)?;
    let low = em.pick(past, last, low)?;
    let next = em.add(low, one)?;
    let over = em.lt(last, next)?;
    let high = em.pick(over, last, next)?;
    let share = em.and_u(place, 0xFF)?;
    let share = em.pick(past, none, share)?;
    Ok((low, high, share))
}

fn blend_em(em: &mut Em<'_>, a: Id, b: Id, share: Id) -> Result<Id, LaneError> {
    let full = em.u(256)?;
    let keep = em.sub(full, share)?;
    let mut out = em.u(0)?;
    for mask in [0x00FF_00FF, 0x0000_FF00] {
        let mask_word = em.u(mask)?;
        let left = em.and(a, mask_word)?;
        let left = em.mul(left, keep)?;
        let right = em.and(b, mask_word)?;
        let right = em.mul(right, share)?;
        let sum = em.add(left, right)?;
        let sum = em.shr(sum, 8)?;
        let sum = em.and(sum, mask_word)?;
        out = em.or(out, sum)?;
    }
    Ok(out)
}

fn covered_em(em: &mut Em<'_>, pixel: Id, over: Id) -> Result<Id, LaneError> {
    let cover = em.shr(over, 24)?;
    let most = em.u(255)?;
    let keep = em.sub(most, cover)?;
    let half = em.u(127)?;
    let mut out = em.u(0)?;
    for shift in [0, 8, 16] {
        let shown = em.shr(pixel, shift)?;
        let shown = em.and_u(shown, 0xFF)?;
        let laid = em.shr(over, shift)?;
        let laid = em.and_u(laid, 0xFF)?;
        let kept = em.mul(shown, keep)?;
        let kept = em.add(kept, half)?;
        let kept = em.div(kept, most)?;
        let value = em.add(laid, kept)?;
        let past = em.lt(most, value)?;
        let value = em.pick(past, most, value)?;
        let value = em.shl(value, shift)?;
        out = em.or(out, value)?;
    }
    Ok(out)
}

/// The pass that stretches the picture over `window` and lays the overlay
/// over it, into the words [`Windowed::of`] gives. Dispatch it over
/// `window.groups(subgroup)`.
pub fn window(build: &Build, window: Screen) -> Result<Vec<u32>, LaneError> {
    refuse("render3::window", build.grid, build.screen)?;
    refuse("render3::window", build.grid, window)?;
    let layout = build.layout();
    let held = Windowed::of(&layout, window);
    let from = build.screen;
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, window)?;
    let none = em.u(0)?;
    let x = em.pick(shown, x, none)?;
    let y = em.pick(shown, y, none)?;
    let (left, right, share_x) = tap_em(&mut em, x, from.wide, window.wide)?;
    let (top, bottom, share_y) = tap_em(&mut em, y, from.high, window.high)?;
    let wide = em.u(from.wide)?;
    let read = |em: &mut Em<'_>, column: Id, row: Id| -> Result<Id, LaneError> {
        let at = em.mul(row, wide)?;
        let at = em.add(at, column)?;
        em.load(PIXELS, at)
    };
    let a = read(&mut em, left, top)?;
    let b = read(&mut em, right, top)?;
    let c = read(&mut em, left, bottom)?;
    let d = read(&mut em, right, bottom)?;
    let upper = blend_em(&mut em, a, b, share_x)?;
    let lower = blend_em(&mut em, c, d, share_x)?;
    let picture = blend_em(&mut em, upper, lower, share_y)?;
    let place = em.pick(shown, pixel, none)?;
    let overlay = em.u(word(held.overlay))?;
    let overlay = em.add(overlay, place)?;
    let over = em.load(PIXELS, overlay)?;
    let value = covered_em(&mut em, picture, over)?;
    let colours = em.u(word(held.colours))?;
    let into = em.add(colours, pixel)?;
    let spare = em.u(word(layout.spare()))?;
    let into = em.pick(shown, into, spare)?;
    em.store(PIXELS, into, value)?;
    kernel.finish()
}

/// The resources of one frame a presenter has in flight.
struct Flight {
    command: vk::CommandBuffer,
    fence: vk::Fence,
    acquired: vk::Semaphore,
    used: bool,
}

/// Why a presenter could not be made or could not present.
#[derive(Debug)]
pub enum Refused {
    /// The host was not made to present, or its queue cannot present to the
    /// window.
    NoPresent,
    /// The surface offers no format the colour words can be copied into
    /// as they are, or no image a copy can write.
    NoFormat,
    /// The surface's size is not the window's.
    Size {
        surface: [u32; 2],
        window: [u32; 2],
    },
    /// The resident holds no `PIXELS`.
    NoPixels,
    Vulkan(vk::Result),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPresent => write!(out, "the device cannot present to the window"),
            Self::NoFormat => write!(
                out,
                "the window's surface offers no B8G8R8A8_UNORM image a copy can write"
            ),
            Self::Size { surface, window } => write!(
                out,
                "the surface is {}x{}, the window {}x{}",
                surface[0], surface[1], window[0], window[1]
            ),
            Self::NoPixels => write!(out, "the resident holds no picture"),
            Self::Vulkan(code) => write!(out, "Vulkan refused: {code:?}"),
        }
    }
}

impl std::error::Error for Refused {}

impl From<vk::Result> for Refused {
    fn from(code: vk::Result) -> Self {
        Self::Vulkan(code)
    }
}

/// How many frames a presenter keeps in flight.
const IN_FLIGHT: usize = 2;

/// How long a presenter waits for an image or for its own work, in
/// nanoseconds.
const PATIENCE: u64 = 5_000_000_000;

/// A swapchain on a Win32 window, fed from the window's words in `PIXELS`.
pub struct Presenter<'h> {
    host: &'h Host,
    surfaces: ash::khr::surface::Instance,
    chains: ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    chain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    done: Vec<vk::Semaphore>,
    pool: vk::CommandPool,
    flights: Vec<Flight>,
    next: usize,
    window: Screen,
    mode: vk::PresentModeKHR,
}

impl<'h> Presenter<'h> {
    /// A presenter on the Win32 window `hwnd` of module `hinstance`, `window`
    /// pixels in size.
    ///
    /// # Safety
    /// `hwnd` and `hinstance` are a live window and the module that made
    /// it, and the window outlives the presenter.
    pub unsafe fn new(
        host: &'h Host,
        hwnd: isize,
        hinstance: isize,
        window: Screen,
    ) -> Result<Self, Refused> {
        if !host.presents() {
            return Err(Refused::NoPresent);
        }
        let entry = host.entry();
        let instance = host.instance();
        let surfaces = ash::khr::surface::Instance::new(entry, instance);
        let win32 = ash::khr::win32_surface::Instance::new(entry, instance);
        let info = vk::Win32SurfaceCreateInfoKHR::default()
            .hwnd(hwnd)
            .hinstance(hinstance);
        let surface = unsafe { win32.create_win32_surface(&info, None) }?;
        let chains = ash::khr::swapchain::Device::new(instance, host.device());
        let mut made = Self {
            host,
            surfaces,
            chains,
            surface,
            chain: vk::SwapchainKHR::null(),
            images: Vec::new(),
            done: Vec::new(),
            pool: vk::CommandPool::null(),
            flights: Vec::new(),
            next: 0,
            window,
            mode: vk::PresentModeKHR::FIFO,
        };
        unsafe { made.build() }?;
        Ok(made)
    }

    /// The present mode the swapchain uses.
    #[must_use]
    pub fn mode(&self) -> &'static str {
        match self.mode {
            vk::PresentModeKHR::MAILBOX => "mailbox",
            vk::PresentModeKHR::IMMEDIATE => "immediate",
            vk::PresentModeKHR::FIFO_RELAXED => "fifo relaxed",
            _ => "fifo",
        }
    }

    unsafe fn build(&mut self) -> Result<(), Refused> {
        let physical = self.host.physical();
        let family = self.host.family();
        let supported = unsafe {
            self.surfaces
                .get_physical_device_surface_support(physical, family, self.surface)
        }?;
        if !supported {
            return Err(Refused::NoPresent);
        }
        let formats = unsafe {
            self.surfaces
                .get_physical_device_surface_formats(physical, self.surface)
        }?;
        let format = formats
            .iter()
            .find(|one| one.format == vk::Format::B8G8R8A8_UNORM)
            .copied()
            .ok_or(Refused::NoFormat)?;
        let caps = unsafe {
            self.surfaces
                .get_physical_device_surface_capabilities(physical, self.surface)
        }?;
        if !caps
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_DST)
        {
            return Err(Refused::NoFormat);
        }
        let extent = vk::Extent2D {
            width: self.window.wide,
            height: self.window.high,
        };
        if caps.current_extent.width != u32::MAX
            && (caps.current_extent.width != extent.width
                || caps.current_extent.height != extent.height)
        {
            return Err(Refused::Size {
                surface: [caps.current_extent.width, caps.current_extent.height],
                window: [extent.width, extent.height],
            });
        }
        let modes = unsafe {
            self.surfaces
                .get_physical_device_surface_present_modes(physical, self.surface)
        }?;
        self.mode = [vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE]
            .into_iter()
            .find(|wanted| modes.contains(wanted))
            .unwrap_or(vk::PresentModeKHR::FIFO);
        let count = if caps.max_image_count == 0 {
            caps.min_image_count + 1
        } else {
            (caps.min_image_count + 1).min(caps.max_image_count)
        };
        let alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|one| caps.supported_composite_alpha.contains(*one))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(self.mode)
            .clipped(true)
            .old_swapchain(self.chain);
        let chain = unsafe { self.chains.create_swapchain(&info, None) }?;
        let old = std::mem::replace(&mut self.chain, chain);
        if old != vk::SwapchainKHR::null() {
            unsafe { self.chains.destroy_swapchain(old, None) };
        }
        self.images = unsafe { self.chains.get_swapchain_images(self.chain) }?;
        let device = self.host.device();
        for semaphore in self.done.drain(..) {
            unsafe { device.destroy_semaphore(semaphore, None) };
        }
        for _ in 0..self.images.len() {
            let made =
                unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?;
            self.done.push(made);
        }
        if self.pool == vk::CommandPool::null() {
            let info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
            self.pool = unsafe { device.create_command_pool(&info, None) }?;
            let info = vk::CommandBufferAllocateInfo::default()
                .command_pool(self.pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(IN_FLIGHT as u32);
            let commands = unsafe { device.allocate_command_buffers(&info) }?;
            for command in commands {
                let fence = unsafe {
                    device.create_fence(
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )
                }?;
                let acquired =
                    unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?;
                self.flights.push(Flight {
                    command,
                    fence,
                    acquired,
                    used: false,
                });
            }
        }
        Ok(())
    }

    /// Waits until no work this presenter sent is left on the device. Call
    /// it before the resident it presents from is dropped.
    pub fn settle(&mut self) -> Result<(), Refused> {
        let device = self.host.device();
        let fences: Vec<vk::Fence> = self
            .flights
            .iter()
            .filter(|flight| flight.used)
            .map(|flight| flight.fence)
            .collect();
        if !fences.is_empty() {
            unsafe { device.wait_for_fences(&fences, true, PATIENCE) }?;
        }
        Ok(())
    }

    /// Copies the window's colours of `held`'s `PIXELS`, which `windowed`
    /// places, into the next image of the swapchain and presents it. Call it
    /// after submitting the frame that writes them: the copy waits on the
    /// device for every pass submitted before it.
    pub fn present(&mut self, held: &Resident<'_>, windowed: Windowed) -> Result<(), Refused> {
        let buffer = held.buffer(PIXELS as usize).ok_or(Refused::NoPixels)?;
        let device = self.host.device();
        let at = self.next % self.flights.len().max(1);
        let Some(flight) = self.flights.get(at) else {
            return Err(Refused::NoPresent);
        };
        let (command, fence, acquired) = (flight.command, flight.fence, flight.acquired);
        unsafe { device.wait_for_fences(&[fence], true, PATIENCE) }?;
        let taken = unsafe {
            self.chains
                .acquire_next_image(self.chain, PATIENCE, acquired, vk::Fence::null())
        };
        let image_at = match taken {
            Ok((image_at, _)) => image_at,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.settle()?;
                unsafe { self.build() }?;
                return Ok(());
            }
            Err(code) => return Err(code.into()),
        };
        let image = self
            .images
            .get(image_at as usize)
            .copied()
            .ok_or(Refused::NoPresent)?;
        let done = self
            .done
            .get(image_at as usize)
            .copied()
            .ok_or(Refused::NoPresent)?;
        unsafe {
            device.reset_fences(&[fence])?;
            device.reset_command_buffer(command, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            record(device, command, buffer, image, windowed);
            device.end_command_buffer(command)?;
        }
        let waits = [acquired];
        let stages = [vk::PipelineStageFlags::TRANSFER];
        let commands = [command];
        let signals = [done];
        let submit = [vk::SubmitInfo::default()
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&commands)
            .signal_semaphores(&signals)];
        let chains = [self.chain];
        let indices = [image_at];
        let shown = vk::PresentInfoKHR::default()
            .wait_semaphores(&signals)
            .swapchains(&chains)
            .image_indices(&indices);
        let presented = {
            let _turn = self.host.ordered();
            unsafe { device.queue_submit(self.host.queue(), &submit, fence) }?;
            unsafe { self.chains.queue_present(self.host.queue(), &shown) }
        };
        if let Some(flight) = self.flights.get_mut(at) {
            flight.used = true;
        }
        self.next = self.next.wrapping_add(1);
        match presented {
            Ok(_) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.settle()?;
                unsafe { self.build() }
            }
            Err(code) => Err(code.into()),
        }
    }
}

/// Records the copy of the window's colours into `image`, between the
/// barriers that order it after the passes before it and before the passes
/// after it, and hand the image to the presentation engine.
unsafe fn record(
    device: &ash::Device,
    command: vk::CommandBuffer,
    buffer: vk::Buffer,
    image: vk::Image,
    windowed: Windowed,
) {
    let range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1);
    let written = [vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(vk::WHOLE_SIZE)];
    let opened = [vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(range)];
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &written,
            &opened,
        );
    }
    let layers = vk::ImageSubresourceLayers::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .layer_count(1);
    let region = [vk::BufferImageCopy::default()
        .buffer_offset(windowed.colours as u64 * 4)
        .buffer_row_length(0)
        .buffer_image_height(0)
        .image_subresource(layers)
        .image_extent(vk::Extent3D {
            width: windowed.window.wide,
            height: windowed.window.high,
            depth: 1,
        })];
    unsafe {
        device.cmd_copy_buffer_to_image(
            command,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &region,
        );
    }
    let read = [vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
        .dst_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::SHADER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(vk::WHOLE_SIZE)];
    let shown = [vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::empty())
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(range)];
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::COMPUTE_SHADER
                | vk::PipelineStageFlags::TRANSFER
                | vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &read,
            &shown,
        );
    }
}

impl Drop for Presenter<'_> {
    fn drop(&mut self) {
        let _ = self.settle();
        let device = self.host.device();
        unsafe {
            for flight in self.flights.drain(..) {
                device.destroy_fence(flight.fence, None);
                device.destroy_semaphore(flight.acquired, None);
            }
            for semaphore in self.done.drain(..) {
                device.destroy_semaphore(semaphore, None);
            }
            if self.pool != vk::CommandPool::null() {
                device.destroy_command_pool(self.pool, None);
            }
            if self.chain != vk::SwapchainKHR::null() {
                self.chains.destroy_swapchain(self.chain, None);
            }
            self.surfaces.destroy_surface(self.surface, None);
        }
    }
}
