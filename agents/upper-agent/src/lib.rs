static mut INPUT: [u8; 1024] = [0; 1024];
static mut INPUT_LEN: usize = 0;
static mut OUTPUT: [u8; 1024] = [0; 1024];
static mut OUTPUT_LEN: usize = 0;

#[no_mangle]
pub extern "C" fn input_ptr() -> *mut u8 {
    unsafe { INPUT.as_mut_ptr() }
}

#[no_mangle]
pub extern "C" fn set_input_len(len: i32) {
    unsafe { INPUT_LEN = len as usize; }
}

#[no_mangle]
pub extern "C" fn process() {
    unsafe {
        // Uppercase the input
        for i in 0..INPUT_LEN {
            OUTPUT[i] = INPUT[i].to_ascii_uppercase();
        }
        OUTPUT_LEN = INPUT_LEN;
    }
}

#[no_mangle]
pub extern "C" fn output_ptr() -> *const u8 {
    unsafe { OUTPUT.as_ptr() }
}

#[no_mangle]
pub extern "C" fn output_len() -> i32 {
    unsafe { OUTPUT_LEN as i32 }
}
