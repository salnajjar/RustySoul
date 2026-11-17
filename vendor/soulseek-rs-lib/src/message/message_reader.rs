use std::io::{self, Read};
use std::{collections::VecDeque, net::TcpStream};
use crate::message::Message;

/// Largest framed payload accepted by a network reader. This preserves the
/// listing inflater's existing 2 GiB ceiling while rejecting larger claims.
pub const MAX_MESSAGE_SIZE: usize =
    crate::utils::zlib::MAX_INFLATED_LISTING + 4;

pub struct MessageReader { buffer:VecDeque<u8> }
impl Default for MessageReader {fn default()->Self{Self::new()}}
impl MessageReader {
 #[must_use] pub const fn new()->Self{Self{buffer:VecDeque::new()}}
 #[cfg(test)] #[must_use] pub fn new_with_buffer(buffer:Vec<u8>)->Self{Self{buffer:buffer.into()}}
 pub fn read_from_socket(&mut self,stream:&mut TcpStream)->io::Result<()> {
  let mut temp_buffer=[0;1024];let bytes_read=stream.read(&mut temp_buffer)?;
  if bytes_read==0{return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"the remote end closed the connection"));}
  self.buffer.extend(&temp_buffer[..bytes_read]);Ok(())
 }
 #[must_use]pub fn buffer_len(&self)->usize{self.buffer.len()}
 pub fn get_buffer(&mut self)->Vec<u8>{self.buffer.drain(..).collect()}
 pub fn extract_message(&mut self)->io::Result<Option<Message>>{
  let bytes_read=self.buffer.len();if bytes_read<4{return Ok(None);}
  let message_size=u32::from_le_bytes([self.buffer[0],self.buffer[1],self.buffer[2],self.buffer[3]]) as usize;
  if message_size > MAX_MESSAGE_SIZE {
   return Err(io::Error::new(io::ErrorKind::InvalidData,"protocol frame exceeds the configured maximum"));
  }
  let total_size=message_size.checked_add(4).ok_or_else(||io::Error::new(io::ErrorKind::InvalidData,"protocol frame length overflow"))?;
  if bytes_read<total_size{return Ok(None);}
  let message_buffer:Vec<u8>=self.buffer.drain(..total_size).collect();Ok(Some(Message::new_with_data(message_buffer)))
 }
}

#[cfg(test)]
mod tests {
    use crate::message::MessageReader;



    #[test]
    fn test_extract_message() {
        let buffer: Vec<u8> = [
            8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101, 8, 0, 0, 0, 112,
            97, 115, 115, 119, 111, 114, 100, 160, 0, 0, 0, 32, 0, 0, 0, 100,
            53, 49, 99, 57, 97, 55, 101, 57, 51, 53, 51, 55, 52, 54, 97, 54,
            48, 50, 48, 102, 57, 54, 48, 50, 100, 52, 53, 50, 57, 50, 57, 17,
            0, 0, 0,
        ]
        .to_vec();
        let mut buffered_reader = MessageReader::new_with_buffer(buffer);
        let mut message = buffered_reader.extract_message().unwrap().unwrap();
        assert_eq!(
            message.get_data(),
            vec![8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101]
        );
        assert_eq!(message.read_string(), "username");
    }
    #[test]
    fn test_extract_message_incomplete_message() {
        let incomplete_buffer = vec![1, 2, 3];
        let mut buffered_reader =
            MessageReader::new_with_buffer(incomplete_buffer);

        let result = buffered_reader.extract_message();
        assert_eq!(None, result.unwrap());

        let rest: Vec<u8> = buffered_reader
            .buffer
            .drain(..buffered_reader.buffer.len())
            .collect();

        assert!(buffered_reader.buffer.is_empty());
        assert_eq!(vec![1, 2, 3], rest);
    }

    #[test]
    fn oversized_network_frame_is_rejected_from_its_header() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut ours = TcpStream::connect(addr).unwrap();
        let (mut theirs, _) = listener.accept().unwrap();
        theirs.write_all(&u32::MAX.to_le_bytes()).unwrap();

        let mut reader = MessageReader::new();
        reader.read_from_socket(&mut ours).unwrap();
        let error = reader.extract_message().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(reader.buffer_len(), 4);
    }

    /// A hangup has to reach the caller as an error. Reported as success it
    /// leaves the actors polling a socket that will never speak again, which is
    /// how a server that closed cleanly went unnoticed.
    #[test]
    fn a_closed_connection_reads_as_an_error_not_as_success() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut ours = TcpStream::connect(addr).unwrap();
        let (mut theirs, _) = listener.accept().unwrap();

        theirs.write_all(&[1, 2, 3]).unwrap();
        theirs.flush().unwrap();
        drop(theirs);

        let mut reader = MessageReader::new();
        // Whatever was sent before the close still arrives.
        while reader.buffer_len() < 3 {
            reader.read_from_socket(&mut ours).unwrap();
        }
        let error = reader
            .read_from_socket(&mut ours)
            .expect_err("a closed connection is not a successful read");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
