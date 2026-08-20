use protocol;

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = protocol::add(2, 2);
        assert_eq!(result, 4);
    }
}