use std::collections::HashMap;

#[derive(Debug)]
pub struct UTXO {
    store: HashMap<Input, Output>,
}

#[derive(PartialEq, Debug)]
pub enum TransactionError {
    InsufficientAmount(TransactionId),
    InvalidInput(TransactionId),
    // InvalidOutput(TransactionId),
    InputNotFound(TransactionId),
    EmptyInput(TransactionId),
}

use self::TransactionError::*;

impl UTXO {

    pub fn get_output(&self, input: &Input) -> Option<&Output> {
        self.store.get(input)
    }

    pub fn new(outs: Vec<Output>) -> UTXO {
        let mut store = HashMap::new();

        for (index, output) in outs.into_iter().enumerate() {
            let input = Input::new(index, TransactionId(0));
            store.insert(input, output);
        }

        UTXO {
            store
        }
    }

    pub fn process_transactions(&mut self, transactions: Vec<Transaction>) -> Result<(), TransactionError> {

        for transaction in transactions.iter() {
            if let Err(err) = self.validate_transaction(transaction) {
                return Err(err)
            }
        }

        for transaction in transactions.into_iter() {
            if let Err(err) = self.process_transaction(transaction) {
                return Err(err)
            };
        }

        Ok(())
    }

    fn validate_transaction(&self, transaction: &Transaction) -> Result<(), TransactionError> {

        if transaction.inputs.is_empty() {
            return Err(EmptyInput(transaction.id))
        }
        
        let mut input_sum = 0;
        for input in transaction.inputs.iter() {
            match self.store.get(input) {
                //Check that inputs are valid
                None => return Err(InputNotFound(transaction.id)),
                Some(output) => input_sum += output.value.0,
            }
        }

        let mut output_sum = 0;

        for output in transaction.outputs.iter() {
            output_sum += output.value.0;
        }

        //Calculate the sum
        if input_sum < output_sum {
            return Err(InsufficientAmount(transaction.id));
        }
    
        Ok(())
    }

    fn process_transaction(&mut self, transaction: Transaction) -> Result<(), TransactionError> {
        //Process inputs
        for input in transaction.inputs.iter() {
            if self.store.remove(&input).is_none() {
                return Err(InputNotFound(transaction.id))
            };
        }

        //Process outputs
        for (index, output) in transaction.outputs.into_iter().enumerate() {
            let input = Input::new(index, transaction.id);
            self.store.insert(input, output);
        }

        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Coin(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Address(String);

type Index = usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TransactionId(u32);

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Input {
    index: Index,
    id: TransactionId,
}

impl Input {
    pub fn new(index: Index, id: TransactionId) -> Input {
        Input {
            index,
            id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    value: Coin,
    address: Address,
}

impl Output {
    pub fn new(value: u64, address: &str) -> Output {
        Output {
            value: Coin(value),
            address: Address(address.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct Transaction {
    id: TransactionId,
    inputs: Vec<Input>,
    outputs: Vec<Output>,
}

impl Transaction {
    pub fn new(id: TransactionId, inputs: Vec<Input>, outputs: Vec<Output>) -> Transaction {
        Transaction {
            id,
            inputs,
            outputs,
        }
    }
}

#[cfg(test)]
mod utxo_test {
    use super::*;
    
    fn initial_outputs() -> Vec<Output> {
        vec![
            Output::new(10000, "Lars"),
            Output::new(100000, "Andres"),
            Output::new(2000000, "Charles"),
            Output::new(10000, "Hiroto")
        ]
    }
    fn test_utxo() -> UTXO {
        UTXO::new(initial_outputs())
    }

    #[test]
    fn should_have_inputs() {
        let utxo = test_utxo();
        let outputs = initial_outputs();

        for (index, output) in outputs.iter().enumerate() {
            let input = Input::new(index, TransactionId(0));
            assert_eq!(utxo.get_output(&input), Some(output));
        } 
    }

    #[test]
    fn process_single_transaction() {

        let mut inputs: Vec<Input> = Vec::new();

        for num in 0..2 {
            let input = Input::new(num, TransactionId(0));
            inputs.push(input);
        }

        let outputs = vec![
            Output::new(1000, "Dog"),
            Output::new(20000, "Cat"),
        ];

        let outputs_copy = outputs.clone();

        let transaction = vec![Transaction::new(TransactionId(1), inputs, outputs)];

        let mut utxo = test_utxo();
        if let Err(_) = utxo.process_transactions(transaction) {
            assert!(false)
        };

        // Check that inputs/outputs are stored in utxo
        for (index, output) in outputs_copy.iter().enumerate() {
            let input = Input::new(index, TransactionId(1));
            assert_eq!(utxo.get_output(&input), Some(output));
        }

        // Check that inputs are spent
        for num in 0..2 {
            let input = Input::new(num, TransactionId(0));
            assert_eq!(utxo.get_output(&input), None);
        }
    }

    #[test]
    fn invalid_input_should_throw_error() {

        const INVALID_TX_ID: TransactionId = TransactionId(10);

        // Here we create inputs that does not exist in UTXO
        let mut inputs: Vec<Input> = Vec::new();
        for num in 0..2 {
            let input = Input::new(num, INVALID_TX_ID);
            inputs.push(input);
        }

        let outputs = vec![
            Output::new(1000, "Dog"),
            Output::new(20000, "Cat"),
        ];

        let transaction = Transaction::new(TransactionId(1), inputs, outputs);

        let mut utxo = test_utxo();

        assert_eq!(
            utxo.process_transactions(vec![transaction]),
            Err(InputNotFound(TransactionId(1)))
            );
    }

    #[test]
    fn insufficient_amount_should_throw_error() {
        let mut inputs: Vec<Input> = Vec::new();

        for num in 0..2 {
            let input = Input::new(num, TransactionId(0));
            inputs.push(input);
        }

        let outputs = vec![
            Output::new(1000000, "Dog"),
            Output::new(2000000, "Cat"),
        ];

        let transaction = Transaction::new(TransactionId(1), inputs, outputs);

        let mut utxo = test_utxo();
        
        assert_eq!(
            utxo.process_transactions(vec![transaction]),
            Err(InsufficientAmount(TransactionId(1)))
        );

    }

    #[test]
    fn empy_input_should_throw_error() {
        let inputs: Vec<Input> = Vec::new();
        let outputs: Vec<Output> = Vec::new();
        let transaction = Transaction::new(TransactionId(1), inputs, outputs);

        let mut utxo = test_utxo();
        assert_eq!(
            utxo.process_transactions(vec![transaction]),
            Err(EmptyInput(TransactionId(1)))
        )
    }

    #[test]
    fn exact_amount_can_be_spent() {
        let mut inputs: Vec<Input> = Vec::new();
        let mut output_sum = 0;
        let mut utxo = test_utxo();
        for num in 0..2 {
            let input = Input::new(num, TransactionId(0));
            output_sum = output_sum + utxo.get_output(&input).unwrap().value.0;
            inputs.push(input);
        }

        let output = Output::new(output_sum, "Water");
        let output_copy = output.clone();
        let transaction = Transaction::new(TransactionId(1), inputs, vec![output]);

        if let Err(_) = utxo.process_transactions(vec![transaction]) {
            assert!(false)
        };

        let input = Input::new(0, TransactionId(1));
        assert_eq!(utxo.get_output(&input), Some(&output_copy));
    }
}