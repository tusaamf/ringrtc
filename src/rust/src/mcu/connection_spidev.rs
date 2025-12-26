use spidev::{SpiModeFlags, Spidev, SpidevOptions, SpidevTransfer};
use std::{io, thread, time::Duration};

use crate::webrtc::peer_connection_factory::McuConfig;

pub const MCU_FIRST_FRAME_DATA: u8 = 0x55;
pub const SPIDEV_BUFFER_SIZE: usize = 256;
// data cho khung encrypt hoặc thông thường không vượt quá 180
pub const MAX_DATA_LENGTH_PER_ENCRYPT_FRAME: usize = 180;
// data cho khung decrypt không vượt quá 216
pub const MAX_DATA_LENGTH_PER_DECRYPT_FRAME: usize = 216;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypeMess {
    CallerEncrypt = 0x83,
    CallerDecrypt = 0x86,
    CalleeEncrypt = 0x85,
    CalleeDecrypt = 0x84,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateMCUFrameMessageError {
    None,
    InvalidDataLength,
}

pub type CreateMCUFrameMessageResult = Result<Vec<u8>, CreateMCUFrameMessageError>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionSpidevError {
    None,
    SPIOpenFailed,
    SPIReceiveFailed,
    MaxRetryExceeded,
    InvalidBufferData,
    CreateFrameFailed,
}

pub type SendMessageToMCUResult = Result<Vec<u8>, ConnectionSpidevError>;

pub struct ConnectionSpidev {
    baudrate_mhz: u8,
    sleep_us: u64,
    retry_quota: u8,
    spidev_path: String,
}

impl ConnectionSpidev {
    pub fn new(baudrate: u8, sleep: u64, quota: u8, path: &str) -> Self {
        Self {
            baudrate_mhz: baudrate,
            sleep_us: sleep,
            retry_quota: quota,
            spidev_path: path.to_string(),
        }
    }

    pub fn from_config(config: &McuConfig) -> Self {
        Self::new(
            config.baudrate,
            config.sleep_us,
            config.retry_quota,
            &config.spidev_path,
        )
    }

    pub fn create_mcu_frame_message(
        &self,
        mess_type: TypeMess,
        data: &[u8],
    ) -> CreateMCUFrameMessageResult {
        info!(
            "TUNT create_mcu_frame_message: baudrate_mhz: {}, mess_type: {}, data: {}, spidev_path: {}",
            self.baudrate_mhz, self.sleep_us, self.retry_quota, self.spidev_path
        );

        // 1. Xác định độ dài tối đa cho phép
        let max_data_length = match mess_type {
            TypeMess::CallerDecrypt | TypeMess::CalleeDecrypt => MAX_DATA_LENGTH_PER_DECRYPT_FRAME,
            _ => MAX_DATA_LENGTH_PER_ENCRYPT_FRAME,
        };

        if data.len() > max_data_length {
            return Err(CreateMCUFrameMessageError::InvalidDataLength);
        }

        let mut message = Vec::with_capacity(SPIDEV_BUFFER_SIZE);

        // 2. Thiết lập Frame ID (2 bytes)
        let frame_id: [u8; 2] = [0x00, 0x01];

        // 3. Tính toán Length (2 bytes), nếu là encrypt thì cộng thêm 4 byte cho phần counter
        let payload_len = match mess_type {
            TypeMess::CalleeEncrypt | TypeMess::CallerEncrypt => (data.len() + 4) as u16,
            _ => data.len() as u16,
        };
        let byte_len = payload_len.to_be_bytes(); // Chuyển int sang 2 bytes (Big Endian)

        // 4. Xây dựng Header (6 bytes đầu)
        message.push(MCU_FIRST_FRAME_DATA); // SOF: 0x55
        message.extend_from_slice(&frame_id); // Frame ID
        message.extend_from_slice(&byte_len); // Length
        message.push(mess_type as u8); // Type Message

        // 5. Tính Header CRC
        let header_cksum_num = self.crc16_arc(&message);
        message.extend_from_slice(&header_cksum_num.to_be_bytes());

        // 6. Xây dựng phần Data và Data CRC
        let mut data_part = Vec::new();

        match mess_type {
            TypeMess::CalleeEncrypt | TypeMess::CallerEncrypt => {
                // Thêm Counter (4 bytes): {0x00, 0x00, 0x00, 0x01}
                let counter: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
                data_part.extend_from_slice(&counter);
                data_part.extend_from_slice(data);
            }
            _ => {
                data_part.extend_from_slice(data);
            }
        }

        // Tính CRC cho phần Data
        let data_cksum_num = self.crc16_arc(&data_part);

        // Gộp Data và Data CRC vào message chính
        message.extend_from_slice(&data_part);
        message.extend_from_slice(&data_cksum_num.to_be_bytes());

        // Implement the logic to create a message frame here
        Ok(message)
    }

    pub fn send_message_to_mcu(&self, buffer: Vec<u8>) -> SendMessageToMCUResult {
        // 1. Kiểm tra độ dài buffer đầu vào (phải <= 256)
        if buffer.len() > SPIDEV_BUFFER_SIZE {
            return Err(ConnectionSpidevError::InvalidBufferData);
        }

        // 2. Khởi tạo và cấu hình thiết bị SPI
        let spi = self
            .create_spi()
            .map_err(|_| ConnectionSpidevError::SPIOpenFailed)?;

        let mut count_read_fail = 0;
        let mut success = false;

        // Chuẩn bị buffer TX cố định 256 bytes
        let mut tx = [0u8; SPIDEV_BUFFER_SIZE];
        let mut rx = [0u8; SPIDEV_BUFFER_SIZE];

        // Copy dữ liệu từ buffer đầu vào vào khung TX
        let copy_len = buffer.len().min(SPIDEV_BUFFER_SIZE);
        tx[..copy_len].copy_from_slice(&buffer[..copy_len]);

        // 3. Vòng lặp thử lại dựa trên retry_quota
        while count_read_fail < self.retry_quota {
            // Xóa sạch buffer RX trước mỗi lần nhận
            rx = [0u8; SPIDEV_BUFFER_SIZE];

            // Thực hiện truyền nhận SPI full-duplex
            let mut transfer = SpidevTransfer::read_write(&tx, &mut rx);
            if spi.transfer(&mut transfer).is_err() {
                return Err(ConnectionSpidevError::SPIReceiveFailed);
            }

            // 4. Kiểm tra dữ liệu phản hồi hợp lệ (2 byte đầu khác 0)
            if rx[0] != 0 || rx[1] != 0 {
                success = true;
                break;
            }

            // 5. Nếu thất bại, ngủ (sleep_us) và tăng biến đếm
            thread::sleep(Duration::from_micros(self.sleep_us));
            count_read_fail += 1;

            info!("SPI/MCU: read failed {} time(s)", count_read_fail);
        }

        // 6. Xử lý kết quả cuối cùng
        if success {
            Ok(rx.to_vec()) // Trả về vector dữ liệu nếu thành công
        } else {
            Err(ConnectionSpidevError::MaxRetryExceeded) // Trả về lỗi nếu quá số lần thử
        }
    }

    fn create_spi(&self) -> io::Result<Spidev> {
        let mut spi = Spidev::open(&self.spidev_path)?;

        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(self.get_baud_rate_hz())
            .mode(SpiModeFlags::SPI_MODE_0)
            .build();

        spi.configure(&options)?;
        Ok(spi)
    }

    fn get_baud_rate_hz(&self) -> u32 {
        match self.baudrate_mhz {
            1 => 1_000_000,
            5 => 5_000_000,
            10 => 10_000_000,
            15 => 15_000_000,
            20 => 20_000_000,
            25 => 25_000_000,
            30 => 30_000_000,
            35 => 35_000_000,
            _ => 1_000_000,
        }
    }

    fn reverse_byte(&self, mut b: u8) -> u8 {
        b = ((b & 0xF0) >> 4) | ((b & 0x0F) << 4);
        b = ((b & 0xCC) >> 2) | ((b & 0x33) << 2);
        b = ((b & 0xAA) >> 1) | ((b & 0x55) << 1);
        b
    }

    fn crc16_arc(&self, data: &[u8]) -> u16 {
        let mut crc: u16 = 0x0000;
        let polynomial: u16 = 0x8005;

        for &byte in data {
            crc ^= (self.reverse_byte(byte) as u16) << 8;
            for _ in 0..8 {
                if (crc & 0x8000) != 0 {
                    crc = ((crc << 1) ^ polynomial) & 0xFFFF;
                } else {
                    crc = (crc << 1) & 0xFFFF;
                }
            }
        }

        let crc_hi = self.reverse_byte(((crc >> 8) & 0xFF) as u8);
        let crc_lo = self.reverse_byte((crc & 0xFF) as u8);

        ((crc_lo as u16) << 8) | (crc_hi as u16)
    }
}
