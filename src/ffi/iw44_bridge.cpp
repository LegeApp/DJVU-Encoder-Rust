// FFI bridge for IW44 encoder
#include "../../../Image/IW44Image.h"
#include "../../../CoreUtils/GPixmap.h"
#include "../../../CoreUtils/GBitmap.h"
#include "../../../IFF/ByteStream.h"
#include <cstdint>
#include <memory>
#include <vector>

using namespace DJVU;

extern "C" {
    // Opaque handle for IW44 encoder
    typedef struct IW44Encoder IW44Encoder;
    
    // Parameters for encoding
    typedef struct {
        int slices;      // Target number of slices 
        int bytes;       // Target file size in bytes
        float decibels;  // Target quality in decibels
    } IW44EncodeParms;
    
    // Create new IW44 encoder from grayscale image
    IW44Encoder* iw44_encoder_new_grayscale(const uint8_t* image_data, 
                                           int width, 
                                           int height,
                                           const uint8_t* mask_data) {
        try {
            // Create GBitmap from input data
            GP<GBitmap> bitmap = GBitmap::create(height, width);
            
            // Copy image data row by row (GBitmap uses different memory layout)
            for (int y = 0; y < height; y++) {
                unsigned char* row = (*bitmap)[y];
                for (int x = 0; x < width; x++) {
                    row[x] = image_data[y * width + x];
                }
            }
            
            // Create mask if provided
            GP<GBitmap> mask;
            if (mask_data) {
                mask = GBitmap::create(height, width);
                for (int y = 0; y < height; y++) {
                    unsigned char* row = (*mask)[y];
                    for (int x = 0; x < width; x++) {
                        row[x] = mask_data[y * width + x];
                    }
                }
            }
            
            // Create IW44 encoder
            GP<IW44Image> encoder = IW44Image::create_encode(*bitmap, mask);
            
            // Return as opaque pointer (increment reference count)
            return reinterpret_cast<IW44Encoder*>(encoder.take());
        } catch (...) {
            return nullptr;
        }
    }
    
    // Create new IW44 encoder from color image  
    IW44Encoder* iw44_encoder_new_color(const uint8_t* image_data,
                                       int width,
                                       int height, 
                                       const uint8_t* mask_data) {
        try {
            // Create GPixmap from input data (assuming RGB format)
            GP<GPixmap> pixmap = GPixmap::create(height, width);
            
            // Copy image data row by row
            for (int y = 0; y < height; y++) {
                GPixel* row = (*pixmap)[y];
                for (int x = 0; x < width; x++) {
                    int idx = (y * width + x) * 3;
                    row[x].r = image_data[idx];
                    row[x].g = image_data[idx + 1]; 
                    row[x].b = image_data[idx + 2];
                }
            }
            
            // Create mask if provided
            GP<GBitmap> mask;
            if (mask_data) {
                mask = GBitmap::create(height, width);
                for (int y = 0; y < height; y++) {
                    unsigned char* row = (*mask)[y];
                    for (int x = 0; x < width; x++) {
                        row[x] = mask_data[y * width + x];
                    }
                }
            }
            
            // Create IW44 encoder
            GP<IW44Image> encoder = IW44Image::create_encode(*pixmap, mask);
            
            return reinterpret_cast<IW44Encoder*>(encoder.take());
        } catch (...) {
            return nullptr;
        }
    }
    
    // Encode chunk with given parameters
    int iw44_encode_chunk(IW44Encoder* encoder, 
                         const IW44EncodeParms* parms,
                         uint8_t** output_data, 
                         size_t* output_size) {
        try {
            if (!encoder || !parms || !output_data || !output_size) {
                return -1;
            }
            
            GP<IW44Image> iw44(reinterpret_cast<IW44Image*>(encoder));
            
            // Create parameters
            IWEncoderParms cpp_parms;
            cpp_parms.slices = parms->slices;
            cpp_parms.bytes = parms->bytes;
            cpp_parms.decibels = parms->decibels;
            
            // Create memory stream to capture output
            GP<ByteStream> stream = ByteStream::create();
            
            // Encode chunk
            int result = iw44->encode_chunk(stream, cpp_parms);
            if (result <= 0) {
                return result;
            }
            
            // Get encoded data
            stream->seek(0);
            size_t data_size = stream->size();
            
            uint8_t* data = (uint8_t*)malloc(data_size);
            if (!data) {
                return -1;
            }
            
            stream->readall(data, data_size);
            
            *output_data = data;
            *output_size = data_size;
            
            return result;
        } catch (...) {
            return -1;
        }
    }
    
    // Get current number of slices encoded
    int iw44_get_slices(IW44Encoder* encoder) {
        try {
            if (!encoder) return -1;
            
            GP<IW44Image> iw44(reinterpret_cast<IW44Image*>(encoder));
            return iw44->get_slices();
        } catch (...) {
            return -1;
        }
    }
    
    // Get current encoded size in bytes  
    int iw44_get_bytes(IW44Encoder* encoder) {
        try {
            if (!encoder) return -1;
            
            GP<IW44Image> iw44(reinterpret_cast<IW44Image*>(encoder));
            return iw44->get_bytes();
        } catch (...) {
            return -1;
        }
    }
    
    // Free encoder
    void iw44_encoder_free(IW44Encoder* encoder) {
        if (encoder) {
            GP<IW44Image> iw44(reinterpret_cast<IW44Image*>(encoder));
            // GP will handle reference counting and cleanup
        }
    }
    
    // Free output data
    void iw44_free_output(uint8_t* data) {
        if (data) {
            free(data);
        }
    }
}
