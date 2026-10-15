//! Numeric pixel kernels only. Geometry validation, cancellation and persistence
//! remain with the host. All dimensions/pointers are supplied by the private JS adapter.
mod surface;
mod ink;
#[no_mangle]
pub extern "C" fn core_abi_version() -> u32 { 1 }

#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    if len == 0 || len > 128 * 1024 * 1024 { return std::ptr::null_mut(); }
    // The same host buffers also carry f32 paper fields and f64 calibration
    // parameters. Their alignment must be guaranteed, not inferred from a
    // particular allocator's usual byte-buffer addresses.
    let layout = std::alloc::Layout::from_size_align(len, 8).unwrap();
    unsafe {
        let pointer = std::alloc::alloc(layout);
        if !pointer.is_null() { std::ptr::write_bytes(pointer, 255, len); }
        pointer
    }
}

/// Pointer/length must be the pair returned by alloc, freed exactly once.
// Native tests must not override libc's one-argument free with this WASM ABI.
#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 && len <= 128 * 1024 * 1024 {
        std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(len, 8).unwrap());
    }
}

// Match Uint8ClampedArray's round-to-nearest, ties-to-even (not f64::round).
fn clamp_byte(value: f64) -> u8 { value.round_ties_even().clamp(0.0, 255.0) as u8 }

#[no_mangle]
pub unsafe extern "C" fn warp_cell(src: *const u8, sw: usize, sh: usize,
    dst: *mut u8, dw: usize, dh: usize, left: f64, right: f64, top: f64, bottom: f64,
    tlx: f64, tly: f64, trx: f64, try_: f64, blx: f64, bly: f64, brx: f64, bry: f64) {
    if sw < 2 || sh < 2 || dw == 0 || dh == 0 || sw > 16384 || sh > 16384 || dw > 16384 || dh > 16384
        || src.is_null() || dst.is_null() || right <= left || bottom <= top
        || ![left,right,top,bottom,tlx,tly,trx,try_,blx,bly,brx,bry].iter().all(|x| x.is_finite()) { return; }
    let input = std::slice::from_raw_parts(src, sw * sh * 4);
    let output = std::slice::from_raw_parts_mut(dst, dw * dh * 4);
    let x0 = (left - 0.5).ceil().clamp(0.0, dw as f64) as usize;
    let x1 = (right - 0.5).ceil().clamp(0.0, dw as f64) as usize;
    let y0 = (top - 0.5).ceil().clamp(0.0, dh as f64) as usize;
    let y1 = (bottom - 0.5).ceil().clamp(0.0, dh as f64) as usize;
    let width = right - left;
    let first_u = (x0 as f64 + 0.5 - left) / width;
    for y in y0..y1 {
        let v = (y as f64 + 0.5 - top) / (bottom - top);
        let lx = tlx + (blx - tlx) * v;
        let ly = tly + (bly - tly) * v;
        let rx = trx + (brx - trx) * v;
        let ry = try_ + (bry - try_) * v;
        let dx = (rx - lx) / width;
        let dy = (ry - ly) / width;
        let mut sx = lx + (rx - lx) * first_u - 0.5;
        let mut sy = ly + (ry - ly) * first_u - 0.5;
        for x in x0..x1 {
            let dest = (y * dw + x) * 4;
            if sx < 0.0 || sy < 0.0 || sx >= (sw - 1) as f64 || sy >= (sh - 1) as f64 {
                output[dest..dest+4].fill(255);
            } else {
                let ix = sx.floor() as usize;
                let iy = sy.floor() as usize;
                let fx = sx - ix as f64;
                let fy = sy - iy as f64;
                let first = (iy * sw + ix) * 4;
                let second = first + sw * 4;
                for c in 0..3 {
                    let upper = input[first+c] as f64 * (1.0-fx) + input[first+4+c] as f64 * fx;
                    let lower = input[second+c] as f64 * (1.0-fx) + input[second+4+c] as f64 * fx;
                    output[dest+c] = clamp_byte(upper * (1.0-fy) + lower * fy);
                }
                output[dest+3] = 255;
            }
            sx += dx; sy += dy;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn split_rgba(src: *const u8, rgb: *mut u8, alpha: *mut u8, pixels: usize) -> u32 {
    let input = std::slice::from_raw_parts(src, pixels * 4);
    let color = std::slice::from_raw_parts_mut(rgb, pixels * 3);
    let mask = std::slice::from_raw_parts_mut(alpha, pixels);
    let mut transparent = 0;
    for p in 0..pixels {
        color[p*3..p*3+3].copy_from_slice(&input[p*4..p*4+3]);
        mask[p] = input[p*4+3];
        transparent |= (mask[p] != 255) as u32;
    }
    transparent
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn clamped_array_rounding() {
        assert_eq!(clamp_byte(12.5), 12); assert_eq!(clamp_byte(13.5), 14);
        assert_eq!(clamp_byte(256.0), 255); assert_eq!(clamp_byte(-1.0), 0);
    }
    #[test] fn alpha_is_lossless() {
        let src = [1,2,3,255, 254,253,252,17]; let mut rgb = [0;6]; let mut alpha = [0;2];
        assert_eq!(unsafe { split_rgba(src.as_ptr(),rgb.as_mut_ptr(),alpha.as_mut_ptr(),2) },1);
        assert_eq!(rgb,[1,2,3,254,253,252]); assert_eq!(alpha,[255,17]);
    }
}

fn yellow(r:f64,g:f64,b:f64)->bool {
    let max=r.max(g).max(b);let min=r.min(g).min(b);let delta=max-min;
    if max<82.0||delta==0.0{return false;}
    let hue=if max==r {60.0*((g-b)/delta%6.0)} else if max==g {60.0*((b-r)/delta+2.0)} else {60.0*((r-g)/delta+4.0)};
    let hue=if hue<0.0{hue+360.0}else{hue};
    (39.0..=68.0).contains(&hue)&&delta/max>=0.16&&g/r.max(1.0)>=0.70&&b/g.max(1.0)<=0.84&&(r+g)*0.5-b>=20.0
}

/// Correct a spatially varying illuminant from paper rings surrounding verified
/// bullet dots. Preserve paper exposure; the colour references still determine
/// pigment calibration in correct_scan_rows. Field planes are R/G and B/G.
#[no_mangle]
pub unsafe extern "C" fn balance_paper_rows(data:*mut u8,width:usize,height:usize,
    white:*const f32,fw:usize,fh:usize,from:usize,to:usize) {
    if data.is_null() || white.is_null() || width==0 || height==0 || fw==0 || fh==0
        || width>16384 || height>16384 || fw>16384 || fh>16384 {return;}
    let out=std::slice::from_raw_parts_mut(data,width*height*4);
    let n=fw*fh;
    let field=std::slice::from_raw_parts(white,n*2);
    let columns:Vec<_>=(0..width).map(|x|{
        let fx=((x as f64+0.5)*fw as f64/width as f64-0.5).clamp(0.0,(fw-1) as f64);
        let ix=fx.floor() as usize;(ix,(ix+1).min(fw-1),fx-ix as f64)
    }).collect();
    for y in from.min(height)..to.min(height) {
        let fy=((y as f64+0.5)*fh as f64/height as f64-0.5).clamp(0.0,(fh-1) as f64);
        let iy=fy.floor() as usize;let ny=(iy+1).min(fh-1);let my=fy-iy as f64;
        for x in 0..width {
            let (ix,nx,mx)=columns[x];
            let sample=|offset:usize| {
                (field[offset+iy*fw+ix] as f64*(1.0-mx)+field[offset+iy*fw+nx] as f64*mx)*(1.0-my)
                    +(field[offset+ny*fw+ix] as f64*(1.0-mx)+field[offset+ny*fw+nx] as f64*mx)*my
            };
            let r=sample(0);let b=sample(n);
            if !r.is_finite() || !b.is_finite() || r<=0.0 || b<=0.0 {continue;}
            let level=r*0.2126+0.7152+b*0.0722;let i=(y*width+x)*4;
            out[i]=clamp_byte(out[i] as f64*level/r);
            out[i+1]=clamp_byte(out[i+1] as f64*level);
            out[i+2]=clamp_byte(out[i+2] as f64*level/b);
        }
    }
}
fn reference_kind(r:f64,g:f64,b:f64,l:f64,s:f64,paper:f64,local:f64)->Option<usize> {
    if yellow(r,g,b){Some(4)}else if r>48.0&&r>g*1.18&&r>b*1.14 {Some(0)}
    else if b>42.0&&b>r*1.13&&b>g*1.06 {Some(2)}else if g>48.0&&g>r*1.1&&g>b*1.16 {Some(3)}
    else if s<=0.2&&l<paper*0.82&&(l<72.0||l<local-(local*0.13).max(22.0)){Some(1)}else{None}
}
fn reference_map(rgb:[f64;3],p:&[f64],kind:usize)->Option<([f64;3],f64)> {
    let base=26+kind*7;if p[base]==0.0{return None;}
    let mut ink=[0.0;3];let mut pixel=[0.0;3];let mut den=0.0;let mut dot=0.0;let mut magnitude=0.0;
    for c in 0..3 {ink[c]=p[base+1+c]*255.0-p[7+c];pixel[c]=rgb[c]-p[7+c];den+=ink[c]*ink[c];dot+=pixel[c]*ink[c];magnitude+=pixel[c]*pixel[c];}
    if den<64.0{return None;}let projection=dot/den;
    if projection<=0.025||projection>2.1{return None;}
    let residual=(0..3).map(|c|{let v=pixel[c]-ink[c]*projection;v*v}).sum::<f64>().sqrt()/magnitude.sqrt().max(18.0);
    if residual>0.5{return None;}
    let amount=projection.clamp(0.0,1.08);let mapped=std::array::from_fn(|c|p[11]+(p[base+4+c]*255.0-p[11])*amount);
    Some((mapped,(1.0-residual/0.5).clamp(0.58,1.0)))
}

/// Shared with the JS fallback: paper field + drawing mask, verified references,
/// exact stencil geometry. Row ranges keep main-thread fallback cancellable.
#[no_mangle]
pub unsafe extern "C" fn correct_scan_rows(data:*mut u8,original:*const u8,width:usize,height:usize,
    field:*const f32,mask:*const u8,fw:usize,fh:usize,parameters:*const f64,from:usize,to:usize,coverage:*mut u8) {
    if data.is_null()||original.is_null()||field.is_null()||parameters.is_null()||width==0||height==0||fw==0||fh==0||width>16384||height>16384{return;}
    let out=std::slice::from_raw_parts_mut(data,width*height*4);
    let raw=std::slice::from_raw_parts(original,width*height*4);
    let light=std::slice::from_raw_parts(field,fw*fh);let p=std::slice::from_raw_parts(parameters,61);
    let mut geometry=if coverage.is_null(){None}else{Some(std::slice::from_raw_parts_mut(coverage,width*height))};
    let drawing=if mask.is_null(){None}else{Some(std::slice::from_raw_parts(mask,fw*fh))};
    let precise=p[0]!=0.0;let stencil=p[1]!=0.0;let observed=p[2]!=0.0;let px=width as f64/21.0;let band=px*0.12;
    let columns:Vec<(usize,usize,f64,u8)>= (0..width).map(|x| {
        let xx=x as f64;let fx=if observed {((xx+0.5)*fw as f64/width as f64-0.5).clamp(0.0,(fw-1) as f64)} else {(xx*((fw-1) as f64/(width-1).max(1) as f64)).round()};
        let ix=fx.floor() as usize;let mix=(fx-ix as f64) as f32 as f64;
        let flags= ((xx-1.5*px).abs()<band||(xx-19.5*px).abs()<band) as u8
            | (((xx<1.5*px-band||xx>19.5*px+band) as u8)<<1)
            | (((xx>=9.92*px&&xx<=11.08*px) as u8)<<2)
            | ((((xx/px*2.0-(xx/px*2.0).round()).abs()<0.24) as u8)<<3);
        (ix,(ix+1).min(fw-1),mix,flags)
    }).collect();
    for y in from.min(height)..to.min(height) {
        let yy=y as f64;let outer_row=(yy-1.43*px).abs()<band||(yy-28.43*px).abs()<band;
        let margin_row=yy<1.43*px-band||yy>28.43*px+band;let marker_row=yy>=27.48*px&&yy<=27.90*px;
        let calibration_row=yy>=27.28*px&&yy<=28.05*px;
        let rail_row=(yy-27.43*px).abs()<=band||(yy-27.93*px).abs()<=band;
        let fy=if observed {((yy+0.5)*fh as f64/height as f64-0.5).clamp(0.0,(fh-1) as f64)} else {(yy*(fh-1) as f64/(height-1).max(1) as f64).round()};
        let iy=fy.floor() as usize;let ny=(iy+1).min(fh-1);let my=fy-iy as f64;
        for x in 0..width {
            let i=(y*width+x)*4;
            if let Some(values)=geometry.as_mut() { values[y*width+x]=0; }
            let r=out[i] as f64;let g=out[i+1] as f64;let b=out[i+2] as f64;let (ix,nx,mx,flags)=columns[x];
            let max=r.max(g).max(b);let min=r.min(g).min(b);let sat=if max>0.0{(max-min)/max}else{0.0};let lum=r*0.2126+g*0.7152+b*0.0722;
            let outer=flags&1!=0||outer_row;
            let margin=flags&2!=0||margin_row;
            let marker=flags&4!=0&&marker_row;
            let synthetic=raw[i]>=254&&raw[i+1]>=254&&raw[i+2]>=254;
            let warm=r>=g*0.85&&g>=r*0.78&&g>=b*1.06&&r>=b*1.12&&(r+g)*0.5-b-(r-g).abs()*0.34>=6.0;
            let vertical=flags&8!=0;
            let rail=outer||(calibration_row&&(rail_row||vertical));
            if (precise&&margin&&(synthetic||lum<105.0||(lum<178.0&&sat>0.22)))
                ||(stencil&&(((outer||calibration_row)&&yellow(r,g,b))||(rail&&warm)||(marker&&sat>0.24))) {
                out[i]=p[11].round() as u8;out[i+1]=out[i];out[i+2]=out[i];continue;
            }
            let mut local=light[iy*fw+ix] as f64;let mut draw=0.0;
            if observed {
                let sample=|values:&[f32]|{let upper=values[iy*fw+ix] as f64*(1.0-mx)+values[iy*fw+nx] as f64*mx;
                    let lower=values[ny*fw+ix] as f64*(1.0-mx)+values[ny*fw+nx] as f64*mx;upper*(1.0-my)+lower*my};
                local=sample(light);
                if let Some(values)=drawing {let upper=values[iy*fw+ix] as f64*(1.0-mx)+values[iy*fw+nx] as f64*mx;
                    let lower=values[ny*fw+ix] as f64*(1.0-mx)+values[ny*fw+nx] as f64*mx;draw=(upper*(1.0-my)+lower*my)/255.0;}
            }
            let deficit=local-lum;
            if let Some(values)=geometry.as_mut() { values[y*width+x]=(255.0*deficit/local.max(40.0)).clamp(0.0,255.0).round() as u8; }
            let has_ink=!observed||deficit>(p[3]*1.5).max(local*0.025);
            let colored=has_ink&&(sat>0.18||(sat>0.105&&deficit>6.0_f64.max(local*0.035)));
            if observed&&!colored&&lum>=local-p[3].max(local*0.0075) {out[i]=255;out[i+1]=255;out[i+2]=255;continue;}
            let neutral=observed&&!colored;let black=neutral&&(lum<local*0.16||(draw>0.85&&lum<local*0.26));
            let automatic=if neutral {[0.0;3]} else {[(r*p[4]).clamp(0.0,255.0),(g*p[5]).clamp(0.0,255.0),(b*p[6]).clamp(0.0,255.0)]};
            let kind=if p[12]!=0.0&&(!neutral||black){reference_kind(r,g,b,lum,sat,p[10],local)}else{None};
            let calibration=kind.and_then(|k|reference_map([r,g,b],p,k));let mut rgb=automatic;
            if !colored&&(observed||calibration.is_none()) {
                let paper=local.max(40.0);let noise=p[3].max(paper*0.0075);let reflectance=(1.0-(paper-lum-noise)/paper).clamp(0.0,1.0);
                let transition=((reflectance-0.12)/0.45).clamp(0.0,1.0);let strength=if observed{(1.25-draw*0.6)*transition*transition*(3.0-2.0*transition)}else{0.65};
                let mut v=p[11]*(reflectance-strength*reflectance*(1.0-reflectance)).clamp(0.0,1.0);
                if black&&kind==Some(1){if let Some((mapped,mix))=calibration {let reference=mapped[0]*0.2126+mapped[1]*0.7152+mapped[2]*0.0722;v+=(reference-v)*if draw>0.85{1.0}else{0.25}*mix;}}
                rgb=[v;3];
            }else if p[13]!=0.0&&colored {
                let paper=local.max(40.0);let input=[(r/paper*p[10]/p[7].max(40.0)).clamp(0.0,1.2),(g/paper*p[10]/p[8].max(40.0)).clamp(0.0,1.2),(b/paper*p[10]/p[9].max(40.0)).clamp(0.0,1.2)];
                for c in 0..3 {let j=14+c*4;rgb[c]=255.0*(p[j]*input[0]+p[j+1]*input[1]+p[j+2]*input[2]+p[j+3]);}
            }else if let Some((mapped,mix))=calibration {for c in 0..3 {rgb[c]=automatic[c]*(1.0-mix)+mapped[c]*mix;}}
            else {let l=automatic[0]*0.2126+automatic[1]*0.7152+automatic[2]*0.0722;for c in 0..3{rgb[c]=l+(automatic[c]-l)*1.08;}}
            if observed&&colored {
                let strongest=rgb[0].max(rgb[1]).max(rgb[2]);let weakest=rgb[0].min(rgb[1]).min(rgb[2]);
                let dilution=1.0-((strongest-weakest)/strongest.max(1.0)).clamp(0.0,1.0);
                let edge=((deficit/local.max(40.0)-0.025)/0.16).clamp(0.0,1.0);
                let edge_strength=edge*edge*(3.0-2.0*edge);
                let vibrance=1.0+0.6*(1.0-draw)*dilution*edge_strength;let tone=rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;
                for c in 0..3 {rgb[c]=tone+(rgb[c]-tone)*vibrance;}
                let coverage=1.0+0.38*(1.0-draw)*edge_strength*((rgb[0].min(rgb[1]).min(rgb[2])-40.0)/90.0).clamp(0.0,1.0);for c in 0..3 {rgb[c]=p[11]-(p[11]-rgb[c])*coverage;}}
            if colored&&max<100.0&&lum<80.0&&r>g&&g>b&&g>=r*0.35 {
                let limit=(automatic[0]*0.2126+automatic[1]*0.7152+automatic[2]*0.0722)*1.1+3.0;
                let mapped=rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;
                if mapped>limit {let scale=limit/mapped;for c in 0..3 {rgb[c]*=scale;}}
            }
            for c in 0..3 {out[i+c]=rgb[c].clamp(0.0,255.0).round() as u8;}
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn separate_ink(data:*mut u8,pixels:usize) {
    if data.is_null(){return;}let out=std::slice::from_raw_parts_mut(data,pixels*4);
    for p in 0..pixels {let i=p*4;let paper=out[i].min(out[i+1]).min(out[i+2]);let alpha=255-paper;
        if alpha==0 {out[i..i+4].fill(0);}else{for c in 0..3 {out[i+c]=(((out[i+c]-paper) as f64*255.0)/alpha as f64).round() as u8;}out[i+3]=alpha;}}
}
