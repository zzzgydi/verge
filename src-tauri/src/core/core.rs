pub trait CoreInterface {
    fn get_path(&self) -> String;
    fn get_name(&self) -> String;
    fn get_version(&self) -> String;

    fn check(&self) -> bool;

    fn start(&self) -> bool;

    fn stop(&self) -> bool;
}
