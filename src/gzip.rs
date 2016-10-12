/// https://tools.ietf.org/html/rfc1952
use std::io;
use std::time;
use std::ffi::CString;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use byteorder::LittleEndian;

use deflate;
use checksum;
use Finish;

const GZIP_ID: [u8; 2] = [31, 139];
const COMPRESSION_METHOD_DEFLATE: u8 = 8;

const OS_FAT: u8 = 0;
const OS_AMIGA: u8 = 1;
const OS_VMS: u8 = 2;
const OS_UNIX: u8 = 3;
const OS_VM_CMS: u8 = 4;
const OS_ATARI_TOS: u8 = 5;
const OS_HPFS: u8 = 6;
const OS_MACINTOSH: u8 = 7;
const OS_Z_SYSTEM: u8 = 8;
const OS_CPM: u8 = 9;
const OS_TOPS20: u8 = 10;
const OS_NTFS: u8 = 11;
const OS_QDOS: u8 = 12;
const OS_ACORN_RISCOS: u8 = 13;
const OS_UNKNOWN: u8 = 255;

const F_TEXT: u8 = 0b000001;
const F_HCRC: u8 = 0b000010;
const F_EXTRA: u8 = 0b000100;
const F_NAME: u8 = 0b001000;
const F_COMMENT: u8 = 0b010000;

#[derive(Debug, Clone)]
pub enum GZipCompressionLevel {
    Fastest,
    Slowest,
    Unknown,
}
impl GZipCompressionLevel {
    fn to_u8(&self) -> u8 {
        match *self {
            GZipCompressionLevel::Fastest => 4,
            GZipCompressionLevel::Slowest => 2,
            GZipCompressionLevel::Unknown => 0,
        }
    }
    fn from_u8(x: u8) -> Self {
        match x {
            4 => GZipCompressionLevel::Fastest,
            2 => GZipCompressionLevel::Slowest,
            _ => GZipCompressionLevel::Unknown,
        }
    }
}
impl From<deflate::CompressionLevel> for GZipCompressionLevel {
    fn from(f: deflate::CompressionLevel) -> Self {
        match f {
            deflate::CompressionLevel::NoCompression |
            deflate::CompressionLevel::BestSpeed => GZipCompressionLevel::Fastest,
            deflate::CompressionLevel::BestCompression => GZipCompressionLevel::Slowest,
            _ => GZipCompressionLevel::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct Trailer {
    crc32: u32,
    input_size: u32,
}
impl Trailer {
    fn read_from<R>(mut reader: R) -> io::Result<Self>
        where R: io::Read
    {
        Ok(Trailer {
            crc32: try!(reader.read_u32::<LittleEndian>()),
            input_size: try!(reader.read_u32::<LittleEndian>()),
        })
    }
    fn write_to<W>(&self, mut writer: W) -> io::Result<()>
        where W: io::Write
    {
        try!(writer.write_u32::<LittleEndian>(self.crc32));
        try!(writer.write_u32::<LittleEndian>(self.input_size));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    modification_time: u32,
    compression_level: GZipCompressionLevel,
    os: Os,
    is_text: bool,
    is_verified: bool,
    extra_field: Option<ExtraField>,
    filename: Option<CString>,
    comment: Option<CString>,
}
impl Default for Header {
    fn default() -> Self {
        let modification_time = time::UNIX_EPOCH.elapsed().map(|d| d.as_secs() as u32).unwrap_or(0);
        Header {
            modification_time: modification_time,
            compression_level: GZipCompressionLevel::Unknown,
            os: Os::Unix,
            is_text: false,
            is_verified: false,
            extra_field: None,
            filename: None,
            comment: None,
        }
    }
}
impl Header {
    pub fn modification_time(&self) -> u32 {
        self.modification_time
    }
    pub fn compression_level(&self) -> GZipCompressionLevel {
        self.compression_level.clone()
    }
    pub fn os(&self) -> Os {
        self.os.clone()
    }
    pub fn is_text(&self) -> bool {
        self.is_text
    }
    pub fn is_verified(&self) -> bool {
        self.is_verified
    }
    pub fn extra_field(&self) -> Option<&ExtraField> {
        self.extra_field.as_ref()
    }
    pub fn filename(&self) -> Option<&CString> {
        self.filename.as_ref()
    }
    pub fn comment(&self) -> Option<&CString> {
        self.comment.as_ref()
    }

    fn flags(&self) -> u8 {
        [(F_TEXT, self.is_text),
         (F_HCRC, self.is_verified),
         (F_EXTRA, self.extra_field.is_some()),
         (F_NAME, self.filename.is_some()),
         (F_COMMENT, self.comment.is_some())]
            .iter()
            .filter(|e| e.1)
            .map(|e| e.0)
            .sum()
    }
    fn crc16(&self) -> u16 {
        let mut crc = checksum::Crc32::new();
        let mut buf = Vec::new();
        Header { is_verified: false, ..self.clone() }.write_to(&mut buf).unwrap();
        crc.update(&buf);
        crc.value() as u16
    }
    fn write_to<W>(&self, mut writer: W) -> io::Result<()>
        where W: io::Write
    {
        try!(writer.write_all(&GZIP_ID));
        try!(writer.write_u8(COMPRESSION_METHOD_DEFLATE));
        try!(writer.write_u8(self.flags()));
        try!(writer.write_u32::<LittleEndian>(self.modification_time));
        try!(writer.write_u8(self.compression_level.to_u8()));
        try!(writer.write_u8(self.os.to_u8()));
        if let Some(ref x) = self.extra_field {
            try!(x.write_to(&mut writer));
        }
        if let Some(ref x) = self.filename {
            try!(writer.write_all(x.as_bytes_with_nul()));
        }
        if let Some(ref x) = self.comment {
            try!(writer.write_all(x.as_bytes_with_nul()));
        }
        if self.is_verified {
            try!(writer.write_u16::<LittleEndian>(self.crc16()));
        }
        Ok(())
    }
    fn read_from<R>(mut reader: R) -> io::Result<Self>
        where R: io::Read
    {
        let mut this = Header::default();
        let mut id = [0; 2];
        try!(reader.read_exact(&mut id));
        if id != GZIP_ID {
            return Err(invalid_data_error!("Unexpected GZIP ID: value={:?}, \
                                                    expected={:?}",
                                           id,
                                           GZIP_ID));
        }
        let compression_method = try!(reader.read_u8());
        if compression_method != COMPRESSION_METHOD_DEFLATE {
            return Err(invalid_data_error!("Compression methods other than DEFLATE(8) are \
                                            unsupported: method={}",
                                           compression_method));
        }
        let flags = try!(reader.read_u8());
        this.modification_time = try!(reader.read_u32::<LittleEndian>());
        this.compression_level = GZipCompressionLevel::from_u8(try!(reader.read_u8()));
        this.os = Os::from_u8(try!(reader.read_u8()));
        if flags & F_EXTRA != 0 {
            this.extra_field = Some(try!(ExtraField::read_from(&mut reader)));
        }
        if flags & F_NAME != 0 {
            this.filename = Some(try!(read_cstring(&mut reader)));
        }
        if flags & F_COMMENT != 0 {
            this.comment = Some(try!(read_cstring(&mut reader)));
        }
        if flags & F_HCRC != 0 {
            let crc = try!(reader.read_u16::<LittleEndian>());
            let expected = this.crc16();
            if crc != expected {
                return Err(invalid_data_error!("CRC16 of GZIP header mismatched: value={}, \
                                                expected={}",
                                               crc,
                                               expected));
            }
            this.is_verified = true;
        }
        Ok(this)
    }
}

fn read_cstring<R>(mut reader: R) -> io::Result<CString>
    where R: io::Read
{
    let mut buf = Vec::new();
    loop {
        let b = try!(reader.read_u8());
        if b == 0 {
            return Ok(unsafe { CString::from_vec_unchecked(buf) });
        }
        buf.push(b);
    }
}

#[derive(Debug, Clone)]
pub struct ExtraField {
    pub id: [u8; 2],
    pub data: Vec<u8>,
}
impl ExtraField {
    fn read_from<R>(mut reader: R) -> io::Result<Self>
        where R: io::Read
    {
        let mut extra = ExtraField {
            id: [0; 2],
            data: Vec::new(),
        };
        try!(reader.read_exact(&mut extra.id));

        let data_size = try!(reader.read_u16::<LittleEndian>()) as usize;
        extra.data.resize(data_size, 0);
        try!(reader.read_exact(&mut extra.data));

        Ok(extra)
    }
    fn write_to<W>(&self, mut writer: W) -> io::Result<()>
        where W: io::Write
    {
        try!(writer.write_all(&self.id));
        try!(writer.write_u16::<LittleEndian>(self.data.len() as u16()));
        try!(writer.write_all(&self.data));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Os {
    Fat,
    Amiga,
    Vms,
    Unix,
    VmCms,
    AtariTos,
    Hpfs,
    Macintosh,
    ZSystem,
    CpM,
    Tops20,
    Ntfs,
    Qdos,
    AcornRiscos,
    Unknown,
    Undefined(u8),
}
impl Os {
    fn to_u8(&self) -> u8 {
        match *self {
            Os::Fat => OS_FAT,
            Os::Amiga => OS_AMIGA,
            Os::Vms => OS_VMS,
            Os::Unix => OS_UNIX,
            Os::VmCms => OS_VM_CMS,
            Os::AtariTos => OS_ATARI_TOS,
            Os::Hpfs => OS_HPFS,
            Os::Macintosh => OS_MACINTOSH,
            Os::ZSystem => OS_Z_SYSTEM,
            Os::CpM => OS_CPM,
            Os::Tops20 => OS_TOPS20,
            Os::Ntfs => OS_NTFS,
            Os::Qdos => OS_QDOS,
            Os::AcornRiscos => OS_ACORN_RISCOS,
            Os::Unknown => OS_UNKNOWN,
            Os::Undefined(os) => os,
        }
    }
    fn from_u8(x: u8) -> Self {
        match x {
            OS_FAT => Os::Fat,
            OS_AMIGA => Os::Amiga,
            OS_VMS => Os::Vms,
            OS_UNIX => Os::Unix,
            OS_VM_CMS => Os::VmCms,
            OS_ATARI_TOS => Os::AtariTos,
            OS_HPFS => Os::Hpfs,
            OS_MACINTOSH => Os::Macintosh,
            OS_Z_SYSTEM => Os::ZSystem,
            OS_CPM => Os::CpM,
            OS_TOPS20 => Os::Tops20,
            OS_NTFS => Os::Ntfs,
            OS_QDOS => Os::Qdos,
            OS_ACORN_RISCOS => Os::AcornRiscos,
            OS_UNKNOWN => Os::Unknown,
            os => Os::Undefined(os),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EncodeOptions {
    header: Header,
    deflate_options: deflate::EncodeOptions,
}
impl EncodeOptions {
    pub fn new() -> Self {
        EncodeOptions::default()
    }
    pub fn modification_time(&mut self, unix_time_secs: u32) -> &mut Self {
        self.header.modification_time = unix_time_secs;
        self
    }
    pub fn text(&mut self) -> &mut Self {
        self.header.is_text = true;
        self
    }
    pub fn verify_header(&mut self) -> &mut Self {
        self.header.is_verified = true;
        self
    }
    pub fn os(&mut self, os: Os) -> &mut Self {
        self.header.os = os;
        self
    }
    pub fn extra_field(&mut self, extra: ExtraField) -> &mut Self {
        self.header.extra_field = Some(extra);
        self
    }
    pub fn filename(&mut self, filename: CString) -> &mut Self {
        self.header.filename = Some(filename);
        self
    }
    pub fn comment(&mut self, comment: CString) -> &mut Self {
        self.header.comment = Some(comment);
        self
    }
    pub fn deflate_options(&mut self, options: deflate::EncodeOptions) -> &mut Self {
        self.header.compression_level = From::from(options.get_level());
        self.deflate_options = options;
        self
    }
}

pub struct Encoder<W> {
    header: Header,
    crc32: checksum::Crc32,
    input_size: u32,
    writer: deflate::Encoder<W>,
}
impl<W> Encoder<W>
    where W: io::Write
{
    pub fn new(inner: W) -> io::Result<Self> {
        Self::with_options(inner, &EncodeOptions::new())
    }
    pub fn with_options(mut inner: W, options: &EncodeOptions) -> io::Result<Self> {
        try!(options.header.write_to(&mut inner));
        Ok(Encoder {
            header: options.header.clone(),
            crc32: checksum::Crc32::new(),
            input_size: 0,
            writer: options.deflate_options.encoder(inner),
        })
    }
    pub fn header(&self) -> &Header {
        &self.header
    }
    pub fn finish(self) -> Finish<W> {
        let trailer = Trailer {
            crc32: self.crc32.value(),
            input_size: self.input_size,
        };
        let mut inner = finish_try!(self.writer.finish());
        match trailer.write_to(&mut inner).and_then(|_| inner.flush()) {
            Ok(_) => Finish::new(inner, None),
            Err(e) => Finish::new(inner, Some(e)),
        }
    }
}
impl<W> io::Write for Encoder<W>
    where W: io::Write
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written_size = try!(self.writer.write(buf));
        self.crc32.update(&buf[..written_size]);
        self.input_size = self.input_size.wrapping_add(written_size as u32);
        Ok(written_size)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[derive(Debug)]
pub struct Decoder<R> {
    header: Header,
    reader: deflate::Decoder<R>,
    crc32: checksum::Crc32,
    eos: bool,
}
impl<R> Decoder<R>
    where R: io::Read
{
    pub fn new(mut inner: R) -> io::Result<Self> {
        let header = try!(Header::read_from(&mut inner));
        Ok(Decoder {
            header: header,
            reader: deflate::Decoder::new(inner),
            crc32: checksum::Crc32::new(),
            eos: false,
        })
    }
    pub fn header(&self) -> &Header {
        &self.header
    }
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }
}
impl<R> io::Read for Decoder<R>
    where R: io::Read
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.eos {
            Ok(0)
        } else {
            let read_size = try!(self.reader.read(buf));
            self.crc32.update(&buf[..read_size]);
            if read_size == 0 {
                self.eos = true;
                let trailer = try!(Trailer::read_from(self.reader.as_inner_mut()));
                if trailer.crc32 != self.crc32.value() {
                    Err(invalid_data_error!("CRC32 mismatched: value={}, expected={}",
                                            self.crc32.value(),
                                            trailer.crc32))
                } else {
                    Ok(0)
                }
            } else {
                Ok(read_size)
            }
        }
    }
}

pub fn decode_all(buf: &[u8]) -> io::Result<Vec<u8>> {
    let mut decoder = Decoder::new(io::Cursor::new(buf)).unwrap();
    let mut buf = Vec::with_capacity(buf.len());
    try!(io::copy(&mut decoder, &mut buf));
    Ok(buf)
}

#[cfg(test)]
mod test {
    use std::io;
    use super::*;

    #[test]
    fn encode_works() {
        let plain = b"Hello World! Hello ZLIB!!";
        let mut encoder = Encoder::new(Vec::new()).unwrap();
        io::copy(&mut &plain[..], &mut encoder).unwrap();
        let encoded = encoder.finish().result().unwrap();
        assert_eq!(decode_all(&encoded).unwrap(), plain);
    }
}
