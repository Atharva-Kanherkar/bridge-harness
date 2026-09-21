"""Verify the encoded master and compare actual movie frames to approved stills."""
from pathlib import Path
import json, subprocess, re
import numpy as np
from PIL import Image
root=Path(__file__).resolve().parent
movie=root/'out/bridge-launch.mp4'
info=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_streams','-show_format','-of','json',str(movie)]))
video=next(s for s in info['streams'] if s['codec_type']=='video')
audio=next(s for s in info['streams'] if s['codec_type']=='audio')
assert(video['width'],video['height'],video['codec_name'])==(1920,1080,'h264')
assert video['r_frame_rate']=='30/1'
assert audio['codec_name']=='aac' and audio['channels']==2
expected=sum(map(int,re.findall(r'seconds:\s*(\d+)',(root/'src/scenes.ts').read_text())))
assert abs(float(info['format']['duration'])-expected)<.1
subprocess.run(['ffmpeg','-v','error','-i',str(movie),'-f','null','-'],check=True)
# Same frame positions as render.mjs --stills.
durations=list(map(int,re.findall(r'seconds:\s*(\d+)',(root/'src/scenes.ts').read_text())))
start=0;comparisons=[]
for i,(seconds,still) in enumerate(zip(durations,sorted((root/'out').glob('[0-9]*.png')))):
 frame=start+int(seconds*30*.72);start+=seconds*30
 raw=subprocess.check_output(['ffmpeg','-v','error','-ss',str(frame/30),'-i',str(movie),'-frames:v','1','-f','rawvideo','-pix_fmt','rgb24','-'])
 actual=np.frombuffer(raw,dtype=np.uint8).reshape(1080,1920,3)
 approved=np.array(Image.open(still).convert('RGB'))
 error=float(np.abs(actual.astype(float)-approved.astype(float)).mean())
 assert error<8, f'{still.name}: unexpectedly different master frame ({error:.2f})'
 if i in [0,1,6,15]:Image.fromarray(actual).save(root/f'out/master-{i+1:02}.png')
 comparisons.append({'scene':still.stem,'frame':frame,'mean_pixel_error':round(error,3)})
result={'duration_seconds':float(info['format']['duration']),'width':1920,'height':1080,'fps':30,'video_codec':'h264','audio_codec':'aac','channels':2,'decode':'passed','frame_comparisons':comparisons,'size_bytes':int(info['format']['size'])}
(root/'out/media-validation.json').write_text(json.dumps(result,indent=2))
print(json.dumps(result,indent=2))
