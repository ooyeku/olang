use olang::ovm::{OlangVirtualMachine, OvmConfig};

fn main() {
    println!("Testing OVM initialization...");
    
    let config = OvmConfig::default();
    println!("Created default config");
    
    match OlangVirtualMachine::new(config) {
        Ok(mut ovm) => {
            println!("✓ OVM created successfully");
            match ovm.start() {
                Ok(()) => println!("✓ OVM started successfully"),
                Err(e) => println!("✗ OVM start failed: {}", e),
            }
        }
        Err(e) => {
            println!("✗ OVM creation failed: {}", e);
        }
    }
}
