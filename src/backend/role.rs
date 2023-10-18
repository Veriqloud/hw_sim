/// Role defines the role that the simulator incarnates during the key exchange.
/// It defaults to [Sender]
#[derive(Debug, PartialEq, Clone)]
#[derive(Default)]
pub enum Role {
    /// Sender in QKD
    #[default]
    Sender,
    /// Receiver in QKD
    Receiver,
    /// Multiparty: tuple arguments are (number of parties, my position)
    OneOfMany(Multiparty),
}



#[derive(Debug, PartialEq, Clone)]
pub struct Multiparty {
    pub number_of_parties: u32,
    /// My position amongst parties
    pub position: u32,
}
