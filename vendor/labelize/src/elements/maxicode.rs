use super::label_position::LabelPosition;
use super::reverse_print::ReversePrint;

#[derive(Clone, Debug)]
pub struct Maxicode {
    pub mode: i32,
}

#[derive(Clone, Debug)]
pub struct MaxicodeWithData {
    pub reverse_print: ReversePrint,
    pub code: Maxicode,
    pub position: LabelPosition,
    pub data: String,
}

impl MaxicodeWithData {
    pub fn get_input_data(&self) -> Result<String, String> {
        const RS: &str = "\x1e";
        const GS: &str = "\x1d";
        let code_header = format!("[)>{RS}01{GS}");
        let header_len = 9;

        let data = &self.data;
        let header_pos = data.find(&code_header);

        let header_pos = match header_pos {
            Some(p) if data.len() >= p + header_len => p,
            _ => return Err("invalid length of maxicode data".to_string()),
        };

        let main_data = &data[..header_pos];
        let add_data = &data[header_pos..];
        let header_data = &add_data[..header_len];

        if main_data.len() < 7 {
            return Err("invalid length of maxicode main data".to_string());
        }

        let class_of_service = &main_data[0..3];
        let ship_to_country = &main_data[3..6];
        let postal_code = &main_data[6..];

        let input_data = add_data.replacen(
            header_data,
            &format!("{header_data}{postal_code}{GS}{ship_to_country}{GS}{class_of_service}{GS}"),
            1,
        );

        Ok(input_data)
    }
}
