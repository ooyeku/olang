use olang::ovm::{OlangVirtualMachine, OvmConfig};

fn main() {
    println!("Testing OVM initialization...");
    
    let config = OvmConfig::default();
    println!("Created default config");
    
    match OlangVirtualMachine::new(config) {
        Ok(mut ovm) => {
            println!("OK OVM created successfully");
            match ovm.start() {
                Ok(()) => println!("OK OVM started successfully"),
                Err(e) => println!("ERROR OVM start failed: {}", e),
            }
        }
        Err(e) => {
            println!("ERROR OVM creation failed: {}", e);
        }
    }
}
