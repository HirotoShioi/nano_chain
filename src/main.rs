use nano_chain::Block;

fn main() {
    println!("Hello, world!");
    let genesis_block = Block::genesis_block();

    println!("Genesis block {:#?}", genesis_block);
}
