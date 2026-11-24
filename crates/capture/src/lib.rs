//! Windows screen capture via DXGI Desktop Duplication API.

use protocol::MonitorInfo;
use thiserror::Error;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO,
};

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("Windows DXGI/D3D11 error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("monitor {0} not found")]
    MonitorNotFound(u32),
    #[error("DXGI desktop duplication access lost (mode switch or secure desktop)")]
    AccessLost,
}

/// Raw captured desktop frame in `BGRA8` pixel format.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Enumerates active desktop monitors across all DXGI adapters.
pub fn list_monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut monitors = Vec::new();
        let mut monitor_id = 0u32;

        let mut adapter_idx = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
            let mut output_idx = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_idx) {
                if let Ok(desc) = output.GetDesc() {
                    if desc.AttachedToDesktop.as_bool() {
                        let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left) as u32;
                        let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top) as u32;
                        let is_primary =
                            desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;
                        let name = String::from_utf16_lossy(
                            &desc.DeviceName[..desc
                                .DeviceName
                                .iter()
                                .position(|&c| c == 0)
                                .unwrap_or(desc.DeviceName.len())],
                        );
                        monitors.push(MonitorInfo {
                            id: monitor_id,
                            name,
                            width,
                            height,
                            is_primary,
                        });
                        monitor_id += 1;
                    }
                }
                output_idx += 1;
            }
            adapter_idx += 1;
        }
        Ok(monitors)
    }
}

/// Captures frames from a single monitor using DXGI Desktop Duplication.
pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    pixel_buf: Vec<u8>,
}

impl DxgiCapturer {
    pub fn new(target_monitor_id: u32) -> Result<Self, CaptureError> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut current_id = 0u32;
            let mut found: Option<(IDXGIAdapter1, IDXGIOutput, u32, u32)> = None;

            let mut adapter_idx = 0u32;
            'outer: while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
                let mut output_idx = 0u32;
                while let Ok(output) = adapter.EnumOutputs(output_idx) {
                    if let Ok(desc) = output.GetDesc() {
                        if desc.AttachedToDesktop.as_bool() {
                            if current_id == target_monitor_id {
                                let w = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left)
                                    as u32;
                                let h = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top)
                                    as u32;
                                found = Some((adapter, output, w, h));
                                break 'outer;
                            }
                            current_id += 1;
                        }
                    }
                    output_idx += 1;
                }
                adapter_idx += 1;
            }

            let (_adapter, output, width, height) =
                found.ok_or(CaptureError::MonitorNotFound(target_monitor_id))?;

            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;

            let device = device.ok_or(CaptureError::MonitorNotFound(target_monitor_id))?;
            let context = context.ok_or(CaptureError::MonitorNotFound(target_monitor_id))?;

            let _dxgi_device: IDXGIDevice = device.cast()?;
            let output1: IDXGIOutput1 = output.cast()?;
            let duplication = output1.DuplicateOutput(&device)?;

            Ok(Self {
                device,
                context,
                duplication,
                staging: None,
                width,
                height,
                pixel_buf: vec![0u8; (width * height * 4) as usize],
            })
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Captures the next changed frame within `timeout_ms`.
    /// Returns `Ok(None)` if the screen did not change before the timeout.
    pub fn capture_frame(
        &mut self,
        timeout_ms: u32,
    ) -> Result<Option<CapturedFrame>, CaptureError> {
        unsafe {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;

            let res = self
                .duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource);

            if let Err(e) = res {
                if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    return Ok(None);
                }
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    return Err(CaptureError::AccessLost);
                }
                return Err(CaptureError::Windows(e));
            }

            let result = (|| -> Result<Option<CapturedFrame>, CaptureError> {
                let Some(res_unwrapped) = resource else {
                    return Ok(None);
                };
                let tex: ID3D11Texture2D = res_unwrapped.cast()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut desc);

                self.width = desc.Width;
                self.height = desc.Height;
                let needed = (desc.Width * desc.Height * 4) as usize;
                if self.pixel_buf.len() != needed {
                    self.pixel_buf.resize(needed, 0);
                    self.staging = None;
                }

                if self.staging.is_none() {
                    let staging_desc = D3D11_TEXTURE2D_DESC {
                        Width: desc.Width,
                        Height: desc.Height,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: desc.SampleDesc,
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                    };
                    let mut st = None;
                    self.device
                        .CreateTexture2D(&staging_desc, None, Some(&mut st))?;
                    self.staging = st;
                }

                let staging = self.staging.as_ref().unwrap();
                let src_res: ID3D11Resource = tex.cast()?;
                let dst_res: ID3D11Resource = staging.cast()?;
                self.context.CopyResource(&dst_res, &src_res);

                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&dst_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

                let row_bytes = (desc.Width * 4) as usize;
                let pitch = mapped.RowPitch as usize;
                let src_ptr = mapped.pData as *const u8;

                for y in 0..desc.Height as usize {
                    let src_row = std::slice::from_raw_parts(src_ptr.add(y * pitch), row_bytes);
                    let dst_start = y * row_bytes;
                    self.pixel_buf[dst_start..dst_start + row_bytes].copy_from_slice(src_row);
                }

                self.context.Unmap(&dst_res, 0);

                Ok(Some(CapturedFrame {
                    width: desc.Width,
                    height: desc.Height,
                    bgra: self.pixel_buf.clone(),
                }))
            })();

            let _ = self.duplication.ReleaseFrame();
            result
        }
    }
}
