const NALU_TYPE_BITMASK: u8 = 0x1F;

pub fn parse_parameter_sets_for_resolution(buf: &[u8]) -> Option<(usize, usize)> {
    // Start past the NAL delimiter
    let mut offset = 'outer: {
        let mut zeroes = 0;
        for (i, &byte) in buf.iter().enumerate() {
            match byte {
                0 => zeroes += 1,
                1 => {
                    if zeroes >= 2 {
                        let candidate = i + 1;
                        // Data is found in the SPS
                        if buf.get(candidate)? & NALU_TYPE_BITMASK == 7 {
                            break 'outer candidate;
                        }
                    }
                    zeroes = 0;
                }
                _ => zeroes = 0,
            }
        }

        // Reached end of buffer
        return None;
    };

    // Skip nal_unit_type
    offset += 1;

    let profile_idc = buf[offset];

    // Skip constraint_sets, level_idc
    offset += 3;

    let mut exp_golomb = ExpGolomb::new(&buf[offset..], 0)?;

    // Skip seq_parameter_set_id
    exp_golomb.skip()?;

    if let 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 13 = profile_idc {
        let chroma_format_idc = exp_golomb.read_unsigned()?;

        if chroma_format_idc == 3 {
            // Skip separate_colour_plane_flag
            exp_golomb.read_single_bit()?;
        }

        // Skip bit_depth_luma_minus8
        exp_golomb.skip()?;
        // Skip bit_depth_chroma_minus8
        exp_golomb.skip()?;

        // Skip qpprime_y_zero_transform_bypass_flag
        exp_golomb.read_single_bit()?;

        let seq_scaling_matrix_present_flag = exp_golomb.read_single_bit()?;
        if seq_scaling_matrix_present_flag == 1 {
            let _count = if chroma_format_idc != 3 { 8 } else { 12 };
            // scaling_list not implemented
            todo!();
        }
    }

    // Skip log2_max_frame_num_minus4
    exp_golomb.skip()?;

    let pic_order_cnt_type = exp_golomb.read_unsigned()?;
    if pic_order_cnt_type == 0 {
        // Skip log2_max_pic_order_cnt_lsb_minus4
        exp_golomb.skip()?;
    } else if pic_order_cnt_type == 1 {
        // Skip delta_pic_order_always_zero_flag
        exp_golomb.read_single_bit()?;
        // Skip offset_for_non_ref_pic
        exp_golomb.skip()?;
        // Skip offset_for_top_to_bottom_field
        exp_golomb.skip()?;

        let num_ref_frames_in_pic_order_cnt_cycle = exp_golomb.read_unsigned()?;
        for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
            // Skip offset_for_ref_frame
            exp_golomb.skip()?;
        }
    }

    // Skip max_num_ref_frames
    exp_golomb.skip()?;
    // Skip gaps_in_frame_num_value_allowed_flag
    exp_golomb.read_single_bit()?;

    let pic_width_in_mbs_minus1 = exp_golomb.read_unsigned()?;
    let pic_height_in_map_units_minus1 = exp_golomb.read_unsigned()?;
    let frame_mbs_only_flag = exp_golomb.read_single_bit()?;

    if frame_mbs_only_flag == 0 {
        // Skip mb_adaptive_frame_field_flag
        exp_golomb.read_single_bit()?;
    }

    // Skip direct_8x8_inference_flag
    exp_golomb.read_single_bit()?;
    let frame_cropping_flag = exp_golomb.read_single_bit()?;

    // These are interpreted as 0 if frame_cropping_flag == 0
    let mut frame_crop_left_offset = 0;
    let mut frame_crop_right_offset = 0;
    let mut frame_crop_top_offset = 0;
    let mut frame_crop_bottom_offset = 0;
    if frame_cropping_flag == 1 {
        frame_crop_left_offset = exp_golomb.read_unsigned()?;
        frame_crop_right_offset = exp_golomb.read_unsigned()?;
        frame_crop_top_offset = exp_golomb.read_unsigned()?;
        frame_crop_bottom_offset = exp_golomb.read_unsigned()?;
    }

    let width = 16 * (pic_width_in_mbs_minus1 + 1)
        - frame_crop_right_offset * 2
        - frame_crop_left_offset * 2;

    let height = 16 * (2 - frame_mbs_only_flag as usize) * (pic_height_in_map_units_minus1 + 1)
        - frame_crop_top_offset * 2
        - frame_crop_bottom_offset * 2;

    return Some((width, height));
}

struct BitIterator<'a> {
    buf: &'a [u8],
    curr_byte: u8,
    index: usize,
    shift_sub: u8,
}

impl<'a> BitIterator<'a> {
    pub fn new(buf: &'a [u8], shift_sub: u8) -> Option<Self> {
        let curr_byte = *buf.get(0)?;
        Some(Self {
            buf,
            curr_byte,
            index: 0,
            shift_sub,
        })
    }
}

impl<'a> std::iter::Iterator for BitIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.shift_sub == 8 {
            self.shift_sub = 0;
            self.index = self.index.wrapping_add(1);
            self.curr_byte = *self.buf.get(self.index)?;
        }
        let shift = 7 - self.shift_sub;
        self.shift_sub += 1;
        let bit = self.curr_byte & (1 << shift);
        Some(bit >> shift)
    }
}

struct ExpGolomb<'a> {
    iter: BitIterator<'a>,
}

impl<'a> ExpGolomb<'a> {
    fn new(buf: &'a [u8], shift_sub: u8) -> Option<Self> {
        BitIterator::new(buf, shift_sub).map(|iter| ExpGolomb { iter })
    }

    fn read_single_bit(&mut self) -> Option<u8> {
        self.iter.next()
    }

    fn count_leading_zeroes(&mut self) -> Option<usize> {
        let mut leading_zeros: usize = 0;
        while let Some(bit) = self.iter.next() {
            if bit == 0 {
                leading_zeros += 1;
            } else {
                return Some(leading_zeros);
            }
        }
        None
    }

    fn skip(&mut self) -> Option<()> {
        let lz = self.count_leading_zeroes()?;
        for _ in 0..lz {
            self.iter.next()?;
        }
        Some(())
    }

    fn read_unsigned(&mut self) -> Option<usize> {
        let mut lz = self.count_leading_zeroes()?;
        let x = (1 << lz) - 1;
        let mut y = 0;

        if lz != 0 {
            while let Some(bit) = self.iter.next() {
                y <<= 1;
                y |= bit as usize;
                lz -= 1;
                if lz == 0 {
                    break;
                }
            }
        }
        return Some(x + y);
    }

    // fn read_signed(&mut self) -> Option<isize> {
    //     self.read_unsigned()
    //         .map(|k| (-1isize).pow((k + 1) as u32) * (k / 2 + k % 2) as isize)
    // }
}

#[test]
fn exp_golomb_count_read_unsigned() {
    let data: Vec<(&[u8], u8, Option<usize>)> = vec![
        (&[0b1], 7, Some(0)),
        (&[0b01000000], 0, Some(1)),
        (&[0b01100000], 0, Some(2)),
        (&[0b00100000], 0, Some(3)),
        (&[0b00101000], 0, Some(4)),
        (&[0b00110000], 0, Some(5)),
        (&[0b00111000], 0, Some(6)),
        (&[0b00011100], 1, Some(6)),
    ];

    for (buf, bit_start, ans) in data {
        let mut exp_golomb = ExpGolomb::new(buf, bit_start).unwrap();
        let res = exp_golomb.read_unsigned();
        assert_eq!(res, ans);
    }

    let buf = [0b10100000];
    let mut exp_golomb = ExpGolomb::new(&buf, 0).unwrap();
    assert_eq!(exp_golomb.read_unsigned(), Some(0));
    assert_eq!(exp_golomb.read_unsigned(), Some(1));

    let buf = [0b01001000];
    let mut exp_golomb = ExpGolomb::new(&buf, 0).unwrap();
    assert_eq!(exp_golomb.read_unsigned(), Some(1));
    assert_eq!(exp_golomb.read_unsigned(), Some(1));
}

#[test]
fn test_parse() {
    const NALU: &'static [u8] = include_bytes!("nalus/csd.bin");
    assert_eq!(
        parse_parameter_sets_for_resolution(NALU),
        Some((1920, 1080))
    );
}
