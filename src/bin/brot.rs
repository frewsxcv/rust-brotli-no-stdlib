mod integration_tests;
extern crate brotli_no_stdlib as brotli;
extern crate core;


#[macro_use]
extern crate alloc_no_stdlib;

use core::ops;
use alloc_no_stdlib::{Allocator, SliceWrapperMut, SliceWrapper,
            StackAllocator, AllocatedStackMemory, bzero};

//use alloc::{SliceWrapper,SliceWrapperMut, StackAllocator, AllocatedStackMemory, Allocator};
use brotli::{BrotliDecompressStream, BrotliState, BrotliResult, HuffmanCode};
pub use brotli::FILE_BUFFER_SIZE;
use std::io::{self, Read, Write, ErrorKind, Error};
use std::time::Duration;
use std::env;

use std::fs::File;

use std::path::Path;

#[cfg(not(feature="disable-timer"))]
use std::time::SystemTime;

#[cfg(feature="disable-timer")]
fn now() -> Duration {
    return Duration::new(0, 0);
}
#[cfg(not(feature="disable-timer"))]
fn now() -> SystemTime {
    return SystemTime::now();
}

#[cfg(not(feature="disable-timer"))]
fn elapsed(start : SystemTime) -> (Duration, bool) {
    match start.elapsed() {
        Ok(delta) => return (delta, false),
        _ => return (Duration::new(0, 0), true),
    }
}

#[cfg(feature="disable-timer")]
fn elapsed(_start : Duration) -> (Duration, bool) {
    return (Duration::new(0, 0), true);
}

declare_stack_allocator_struct!(MemPool, 4096, calloc);


fn _write_all<OutputType> (w : &mut OutputType, buf : &[u8]) -> Result<(), io::Error>
where OutputType: Write {
    let mut total_written : usize = 0;
    while total_written < buf.len() {
        match w.write(&buf[total_written..]) {
            Err(e) => {
                match e.kind() {
                    ErrorKind::Interrupted => continue,
                    _ => return Err(e),
                }
            },
            Ok(cur_written) => {
                if cur_written == 0 {
                     return Err(Error::new(ErrorKind::UnexpectedEof, "Write EOF"));
                }
                total_written += cur_written;
            }
        }
    }
    Ok(())
}

//trace_macros!(true);

pub fn decompress<InputType, OutputType> (r : &mut InputType, mut w : &mut OutputType) -> Result<(), io::Error>
where InputType: Read, OutputType: Write {
    return decompress_internal(r, w, 4096 * 1024, 4096 * 1024);
}
pub fn decompress_internal<InputType, OutputType> (r : &mut InputType, mut w : &mut OutputType, input_buffer_limit : usize, output_buffer_limit : usize) -> Result<(), io::Error>
where InputType: Read, OutputType: Write {
  let mut total = Duration::new(0, 0);
  let range : usize;
  let mut timing_error : bool = false;
  if option_env!("BENCHMARK_MODE").is_some() {
    range = 1000;
  } else {
    range = 1;
  }
  for _i in 0..range {
    define_allocator_memory_pool!(calloc_u8_buffer, 4096, u8, [0; 32 * 1024 * 1024], calloc);
    define_allocator_memory_pool!(calloc_u32_buffer, 4096, u32, [0; 4 * 1024 * 1024], calloc);
    define_allocator_memory_pool!(calloc_hc_buffer, 4096, HuffmanCode, [0; 8 * 1024 * 1024], calloc);
    let calloc_u8_allocator = MemPool::<u8>::new_allocator(calloc_u8_buffer, bzero);
    let calloc_u32_allocator = MemPool::<u32>::new_allocator(calloc_u32_buffer, bzero);
    let calloc_hc_allocator = MemPool::<HuffmanCode>::new_allocator(calloc_hc_buffer, bzero);
    //test(calloc_u8_allocator);
    let mut brotli_state = BrotliState::new(calloc_u8_allocator, calloc_u32_allocator, calloc_hc_allocator);
    let mut input = brotli_state.alloc_u8.alloc_cell(input_buffer_limit);
    let mut output = brotli_state.alloc_u8.alloc_cell(output_buffer_limit);
    let mut available_out : usize = output.slice().len();

    //let amount = try!(r.read(&mut buf));
    let mut available_in : usize = 0;
    let mut input_offset : usize = 0;
    let mut output_offset : usize = 0;
    let mut result : BrotliResult = BrotliResult::NeedsMoreInput;
    loop {
      match result {
          BrotliResult::NeedsMoreInput => {
              input_offset = 0;
              match r.read(input.slice_mut()) {
                  Err(e) => {
                      match e.kind() {
                          ErrorKind::Interrupted => continue,
                          _ => return Err(e),
                      }
                  },
                  Ok(size) => {
                      if size == 0 {
                          return Err(Error::new(ErrorKind::UnexpectedEof, "Read EOF"));
                      }
                      available_in = size;
                  },
              }
          },
          BrotliResult::NeedsMoreOutput => {
              try!(_write_all(&mut w, &output.slice()[..output_offset]));
              output_offset = 0;
          },
          BrotliResult::ResultSuccess => break,
          BrotliResult::ResultFailure => panic!("FAILURE"),
      }
      let mut written :usize = 0;
      let start = now();
      result = BrotliDecompressStream(&mut available_in, &mut input_offset, &input.slice(),
                                      &mut available_out, &mut output_offset, &mut output.slice_mut(),
                                      &mut written, &mut brotli_state);

      let (delta, err) = elapsed(start);
      if err {
          timing_error = true;
      }
      total = total + delta;
      if output_offset != 0 {
          try!(_write_all(&mut w, &output.slice()[..output_offset]));
          output_offset = 0;
          available_out = output.slice().len()
      }
    }
    brotli_state.BrotliStateCleanup();
  }
  if timing_error {
      let _r = writeln!(&mut std::io::stderr(), "Timing error\n");
  } else {
      let _r = writeln!(&mut std::io::stderr(), "Time {:}.{:09}\n",
                        total.as_secs(),
                        total.subsec_nanos());
  }
  Ok(())
}

fn main() {
    if env::args_os().len() > 1 {
        let mut first = true;
        for argument in env::args() {
            if first {
               first = false;
               continue;
            }
            let mut input = match File::open(&Path::new(&argument)) {
                Err(why) => panic!("couldn't open {}: {:?}", argument,
                                                       why),
                Ok(file) => file,
            };
            let oa = argument + ".original";
            let mut output = match File::create(&Path::new(&oa), ) {
                Err(why) => panic!("couldn't open file for writing: {:} {:?}", oa, why),
                Ok(file) => file,
            };
            match decompress(&mut input, &mut output) {
                Ok(_) => {},
                Err(e) => panic!("Error {:?}", e),
            }
            drop(output);
            drop(input);
        }
    } else {
        match decompress(&mut io::stdin(), &mut io::stdout()) {
            Ok(_) => return,
            Err(e) => panic!("Error {:?}", e),
        }
    }
}
